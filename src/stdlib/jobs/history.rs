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
pub(super) fn save(handle: &Value, key: &str, data: HashMap<String, Value>) -> Result<()> {
    let ttl = JOB_RUNTIME
        .history_retention
        .read()
        .map_err(|_| IntentError::runtime_error("job history configuration lock poisoned"))?
        .ttl(&data);
    kv::kv_set(handle, key, &Value::Map(data), ttl)
}
