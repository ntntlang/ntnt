//! Durable-job policy. Byte limits account for serialized records, not backend pages/RSS.
use super::*;

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Policy {
    pub enabled: bool,
    pub completed_days: i64,
    pub failed_days: i64,
    pub max_records: i64,
    pub max_bytes: i64,
    pub batch_size: usize,
    pub interval_secs: u64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            completed_days: 30,
            failed_days: 90,
            max_records: 10_000_000,
            max_bytes: 21_474_836_480,
            batch_size: 128,
            interval_secs: 60,
        }
    }
}
/// One coordinator per process/store URL, shared by worker guards. No coordinator
/// is started by configure/inspect/list. Redis maintenance uses a separate, bounded-I/O connection.
static COORDINATORS: LazyLock<Mutex<HashMap<String, std::sync::Weak<Coordinator>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub(super) struct Coordinator {
    stop: Arc<(Mutex<bool>, std::sync::Condvar)>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        let (lock, wake) = &*self.stop;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wake.notify_all();
        if let Some(t) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // Never hold worker shutdown hostage to network or filesystem I/O.
            // Redis batches have their own deadline; the stop flag fences future batches.
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}
pub(super) fn acquire(info: &KvHandleInfo) -> Option<Arc<Coordinator>> {
    if JOB_RUNTIME
        .test_queue
        .lock()
        .map(|q| q.is_some())
        .unwrap_or(true)
    {
        return None;
    }
    let mut all = COORDINATORS.lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|_, v| v.strong_count() > 0);
    if let Some(existing) = all.get(&info.url).and_then(|w| w.upgrade()) {
        return Some(existing);
    }
    let stop = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let state = stop.clone();
    let info = info.clone();
    let name = info.url.clone();
    let thread = std::thread::Builder::new()
        .name("ntnt-job-retention".into())
        .spawn(move || {
            let handle = info.to_value();
            loop {
                if *state.0.lock().unwrap_or_else(|e| e.into_inner()) {
                    break;
                }
                let mut wait = std::time::Duration::from_secs(60);
                match job_store::maintenance::step(&handle, now_ms()) {
                    Ok((n, more, p)) => {
                        wait = std::time::Duration::from_secs(p.interval_secs);
                        if n > 0 {
                            emit_job_event(
                                "jobs.retention.pruned",
                                &[
                                    ("records", Value::Int(n as i64)),
                                    ("batch_size", Value::Int(p.batch_size as i64)),
                                ],
                            );
                        }
                        // Catch-up yields between bounded transactions.
                        if more {
                            wait = std::time::Duration::from_millis(10);
                        }
                    }
                    Err(_) => emit_job_event(
                        "jobs.retention.error",
                        &[("operation", Value::String("maintenance".into()))],
                    ),
                }
                let guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
                if *guard {
                    break;
                }
                let _ = state.1.wait_timeout(guard, wait);
            }
        })
        .map_err(|_| {
            emit_job_event(
                "jobs.retention.error",
                &[("operation", Value::String("coordinator_start".into()))],
            )
        })
        .ok()?;
    let coordinator = Arc::new(Coordinator {
        stop,
        thread: Mutex::new(Some(thread)),
    });
    all.insert(name, Arc::downgrade(&coordinator));
    Some(coordinator)
}
pub(super) fn wake() {
    for c in COORDINATORS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter_map(|w| w.upgrade())
    {
        c.stop.1.notify_all();
    }
}
#[cfg(test)]
pub(super) fn reset() {
    let coordinators: Vec<_> = COORDINATORS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
        .filter_map(|(_, w)| w.upgrade())
        .collect();
    for c in coordinators {
        *c.stop.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        c.stop.1.notify_all();
        if let Some(t) = c.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // Never hold worker shutdown hostage to network or filesystem I/O.
            // Redis batches have their own deadline; the stop flag fences future batches.
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}

#[cfg(test)]
mod coordinator_tests {
    use super::*;
    #[test]
    fn coordinator_drop_does_not_wait_for_blocked_io() {
        let (release, blocked) = std::sync::mpsc::channel();
        let (done, completed) = std::sync::mpsc::channel();
        let coordinator = Coordinator {
            stop: Arc::new((Mutex::new(false), std::sync::Condvar::new())),
            thread: Mutex::new(Some(std::thread::spawn(move || {
                let _ = blocked.recv();
            }))),
        };
        let dropper = std::thread::spawn(move || {
            drop(coordinator);
            let _ = done.send(());
        });
        let returned = completed.recv_timeout(std::time::Duration::from_secs(1));
        let _ = release.send(());
        dropper.join().unwrap();
        assert!(
            returned.is_ok(),
            "worker shutdown waited for blocked maintenance I/O"
        );
    }
}

impl Policy {
    pub fn parse(value: Option<&Value>) -> Result<Self> {
        let mut p = Self::default();
        let Some(value) = value else { return Ok(p) };
        let Value::Map(m) = value else {
            return Err(IntentError::type_error("retention must be a map"));
        };
        for (key, value) in m {
            if key == "enabled" {
                let Value::Bool(v) = value else {
                    return Err(IntentError::type_error(
                        "retention.enabled must be a boolean",
                    ));
                };
                p.enabled = *v;
                continue;
            }
            // Bounded settings also keep all Redis integer accounting below 2^53.
            let max = match key.as_str() {
                "completed_days" | "failed_days" => 365_000,
                "max_records" => 1_000_000_000,
                "max_bytes" => 1_i64 << 50,
                "batch_size" => 4096,
                "interval_secs" => 86400,
                _ => return Err(IntentError::type_error("unknown retention setting")),
            };
            let Value::Int(v) = value else {
                return Err(IntentError::type_error(format!(
                    "retention.{key} must be a positive integer"
                )));
            };
            if *v < 1 || *v > max {
                return Err(IntentError::type_error(format!(
                    "retention.{key} must be between 1 and {max}"
                )));
            }
            match key.as_str() {
                "completed_days" => p.completed_days = *v,
                "failed_days" => p.failed_days = *v,
                "max_records" => p.max_records = *v,
                "max_bytes" => p.max_bytes = *v,
                "batch_size" => p.batch_size = *v as usize,
                "interval_secs" => p.interval_secs = *v as u64,
                _ => unreachable!(),
            }
        }
        Ok(p)
    }
}
