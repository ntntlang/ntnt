//! Age-based history retention uses the KV backend's existing per-key TTL.
//! No job-history index, eviction queue, sweep or persisted policy is needed.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct HistoryRetention {
    pub completed_secs: Option<i64>,
    pub failed_secs: Option<i64>,
}

impl Default for HistoryRetention {
    fn default() -> Self {
        Self {
            completed_secs: Some(30 * 86400),
            failed_secs: Some(90 * 86400),
        }
    }
}

impl HistoryRetention {
    pub fn parse(value: Option<&Value>) -> Result<Self> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let Value::Map(opts) = value else {
            return Err(IntentError::type_error("retention must be a map"));
        };
        let mut policy = Self::default();
        let mut enabled = true;
        for (key, value) in opts {
            match (key.as_str(), value) {
                ("enabled", Value::Bool(b)) => enabled = *b,
                ("completed_days" | "failed_days", Value::Int(n)) if (1..=365000).contains(n) => {
                    let ttl = Some(n * 86400);
                    if key == "completed_days" {
                        policy.completed_secs = ttl;
                    } else {
                        policy.failed_secs = ttl;
                    }
                }
                _ => return Err(IntentError::runtime_error(
                    "retention accepts only enabled (boolean), completed_days and failed_days (integers 1..365000)"
                )),
            }
        }
        if !enabled {
            policy.completed_secs = None;
            policy.failed_secs = None;
        }
        Ok(policy)
    }

    fn ttl(&self, data: &HashMap<String, Value>) -> Option<i64> {
        match data.get("status") {
            Some(Value::String(s)) => match s.as_str() {
                "completed" | "cancelled" => self.completed_secs,
                "dead" | "failed" | "expired" => self.failed_secs,
                _ => None,
            },
            _ => None,
        }
    }
}

/// One SET saves both state and expiration. Returning to a live state removes
/// the TTL in the same write, so expiration cannot delete a newly retried job.
#[cfg(test)]
pub(super) fn save(handle: &Value, key: &str, data: HashMap<String, Value>) -> Result<()> {
    let ttl = JOB_RUNTIME
        .history_retention
        .read()
        .map_err(|_| IntentError::runtime_error("job history configuration lock poisoned"))?
        .ttl(&data);
    kv::kv_set(handle, key, &Value::Map(data), ttl)
}

pub(super) struct Prepared {
    pub next: kv::conditional::Snapshot,
    expected: Option<kv::conditional::Snapshot>,
    key: String,
    owner: String,
    ttl: Option<i64>,
    remove: Vec<String>,
    pending: Option<String>,
}
impl Prepared {
    pub fn new(
        handle: &Value,
        key: &str,
        expected: Option<kv::conditional::Snapshot>,
        mut data: HashMap<String, Value>,
    ) -> Result<Self> {
        let field = |name: &str| match data.get(name) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let owner = field("id");
        let status = field("status");
        let pending = if matches!(status.as_str(), "pending" | "scheduled" | "retrying") {
            Some(field("pending_key"))
                .filter(|k| k.starts_with("jobs:pending:") && k.ends_with(&format!(":{owner}")))
        } else {
            None
        };
        let mut remove = Vec::new();
        if let Some(Value::Map(old)) = expected.as_ref().map(|s| s.value()) {
            if let Some(Value::String(k)) = old.get("pending_key") {
                remove.push(k.clone());
            }
        }
        if status != "active" {
            remove.push(format!("jobs:active:{owner}"));
        }
        if matches!(status.as_str(), "cancelled" | "dead" | "failed" | "expired") {
            let key = field("dedup_key");
            if key.starts_with("jobs:unique:") {
                remove.push(key);
            }
        }
        if matches!(
            status.as_str(),
            "pending" | "scheduled" | "retrying" | "failed"
        ) {
            for key in [
                "claim_token",
                "worker_id",
                "execution_phase",
                "lease_expires_at_ms",
                "_lease_ready_status",
            ] {
                data.remove(key);
            }
        }
        let ttl = JOB_RUNTIME
            .history_retention
            .read()
            .map_err(|_| IntentError::runtime_error("job history configuration lock poisoned"))?
            .ttl(&data);
        // One ID per prepared mutation, stable across acknowledgement loss.
        data.insert(
            "_job_write_id".into(),
            Value::String(Uuid::new_v4().to_string()),
        );
        let next = kv::conditional::Snapshot::prepare(handle, &Value::Map(data))?;
        Ok(Self {
            next,
            expected,
            key: key.into(),
            owner,
            ttl,
            remove,
            pending,
        })
    }
    pub fn apply(&self, handle: &Value) -> Result<bool> {
        kv::conditional::write(
            handle,
            &self.key,
            self.expected.as_ref(),
            &self.next,
            self.ttl,
            &self.owner,
            &self.remove,
            self.pending.as_deref(),
            false,
        )
    }
    pub fn apply_owned(&self, handle: &Value) -> Result<bool> {
        kv::conditional::write(
            handle,
            &self.key,
            self.expected.as_ref(),
            &self.next,
            self.ttl,
            &self.owner,
            &self.remove,
            self.pending.as_deref(),
            true,
        )
    }
    pub fn retry(&self, handle: &Value) -> Result<bool> {
        retry_storage(&self.owner, || self.apply_owned(handle))
    }
}

/// Retry storage only: callers retain the prepared result, never rerun perform
/// or on_failure. Interruption during an outage needs operator reconciliation.
pub(super) fn retry_storage<T>(id: &str, mut action: impl FnMut() -> Result<T>) -> Result<T> {
    let mut warned = false;
    loop {
        match action() {
            Ok(value) => return Ok(value),
            Err(error) => {
                if !warned {
                    eprintln!("[ntnt] job '{id}': storage unavailable; retrying persistence only");
                    warned = true;
                }
                if sleep_cancellable(std::time::Duration::from_millis(100)) {
                    eprintln!("[ntnt] job '{id}': persistence interrupted; reconcile durable state before replaying work");
                    return Err(error);
                }
            }
        }
    }
}

pub(super) fn worker_write(
    handle: &Value,
    key: &str,
    expected: &mut Option<kv::conditional::Snapshot>,
    data: &HashMap<String, Value>,
) -> bool {
    let result: Result<bool> = (|| {
        let prepared = Prepared::new(handle, key, expected.clone(), data.clone())?;
        if prepared.retry(handle)? {
            *expected = Some(prepared.next);
            Ok(true)
        } else {
            Ok(false)
        }
    })();
    match result {
        Ok(applied) => applied,
        Err(error) => {
            eprintln!("[ntnt] job state was not persisted: {error}");
            false
        }
    }
}
