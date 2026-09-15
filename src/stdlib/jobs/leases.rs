//! Worker-local lease supervision. No jobs or task-history entries are spawned.
use super::*;
use std::sync::Condvar;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct Policy {
    pub duration_ms: i64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            duration_ms: 300_000,
        }
    }
}
impl Policy {
    pub fn parse(value: Option<&Value>) -> Result<Self> {
        match value {
            None => Ok(Self::default()),
            Some(Value::Int(seconds)) if (10..=86400).contains(seconds) => Ok(Self {
                duration_ms: seconds * 1000,
            }),
            _ => Err(IntentError::type_error(
                "lease_seconds must be an integer from 10 through 86400",
            )),
        }
    }
    fn renewal_interval(self) -> Duration {
        Duration::from_millis((self.duration_ms / 3).min(30_000).max(1) as u64)
    }
}

#[derive(Clone)]
struct Assignment {
    id: String,
    token: String,
    cancel: Arc<CancelToken>,
    policy: Policy,
}
#[derive(Default)]
struct State {
    stopped: bool,
    assignment: Option<Assignment>,
}

pub(super) struct Keeper {
    shared: Arc<(Mutex<State>, Condvar)>,
}
impl Keeper {
    pub fn new(info: KvHandleInfo) -> Result<Self> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let state = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("ntnt-job-lease".into())
            .spawn(move || {
                let handle = info.to_value();
                let mut previous = String::new();
                let mut next = Instant::now();
                loop {
                    let (lock, wake) = &*state;
                    let mut current = lock.lock().unwrap_or_else(|e| e.into_inner());
                    if current.stopped {
                        break;
                    }
                    let Some(assignment) = current.assignment.clone() else {
                        drop(wake.wait(current).unwrap_or_else(|e| e.into_inner()));
                        continue;
                    };
                    if assignment.token != previous {
                        previous = assignment.token.clone();
                        next = Instant::now() + assignment.policy.renewal_interval();
                    }
                    let remaining = next.saturating_duration_since(Instant::now());
                    if !remaining.is_zero() {
                        drop(
                            wake.wait_timeout(current, remaining)
                                .unwrap_or_else(|e| e.into_inner()),
                        );
                        continue;
                    }
                    if assignment.cancel.is_cancelled() {
                        current.assignment = None;
                        continue;
                    }
                    drop(current);
                    let sent = Instant::now();
                    match kv::job_leases::renew(
                        &handle,
                        &assignment.id,
                        &assignment.token,
                        assignment.policy.duration_ms,
                    ) {
                        Ok(true) => {
                            assignment.cancel.renew_deadline(
                                sent + Duration::from_millis(assignment.policy.duration_ms as u64),
                            );
                        }
                        Ok(false) => assignment.cancel.cancel(),
                        Err(_) => { /* The independent monotonic deadline still expires. */ }
                    }
                    next = Instant::now() + assignment.policy.renewal_interval();
                }
            })
            .map_err(|e| {
                IntentError::runtime_error(format!("cannot start job lease supervisor: {e}"))
            })?;
        Ok(Self { shared })
    }

    pub fn attach(&self, claim: &kv::job_leases::Claim, sent: Instant, policy: Policy) -> Scope {
        let previous = CURRENT_CANCEL_TOKEN.with(|c| c.borrow().clone());
        // Starting the deadline at send-time is conservative about RPC latency.
        let remaining = claim
            .deadline_ms
            .saturating_sub(claim.observed_now_ms)
            .max(0) as u64;
        let cancel = Arc::new(CancelToken::with_parent_deadline(
            previous.clone(),
            sent + Duration::from_millis(remaining),
        ));
        CURRENT_CANCEL_TOKEN.with(|c| *c.borrow_mut() = Some(Arc::clone(&cancel)));
        let (lock, wake) = &*self.shared;
        lock.lock().unwrap_or_else(|e| e.into_inner()).assignment = Some(Assignment {
            id: claim.id.clone(),
            token: claim.token.clone(),
            cancel: Arc::clone(&cancel),
            policy,
        });
        wake.notify_all();
        Scope {
            shared: Arc::clone(&self.shared),
            token: claim.token.clone(),
            cancel,
            previous,
            attached: true,
        }
    }
}
impl Drop for Keeper {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.stopped = true;
        if let Some(a) = state.assignment.take() {
            a.cancel.cancel();
        }
        wake.notify_all();
        // No join on a potentially stalled filesystem operation during shutdown.
        // The one supervisor exits after its in-flight renewal returns.
    }
}

pub(super) struct Scope {
    shared: Arc<(Mutex<State>, Condvar)>,
    token: String,
    pub cancel: Arc<CancelToken>,
    previous: Option<Arc<CancelToken>>,
    attached: bool,
}
impl Scope {
    pub fn detach(&mut self) {
        if !self.attached {
            return;
        }
        self.attached = false;
        self.cancel.cancel();
        CURRENT_CANCEL_TOKEN.with(|c| *c.borrow_mut() = self.previous.take());
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .assignment
            .as_ref()
            .is_some_and(|a| a.token == self.token)
        {
            state.assignment = None;
        }
        wake.notify_all();
    }
}

pub(super) fn ready_data(data: &HashMap<String, Value>) -> HashMap<String, Value> {
    let mut ready = data.clone();
    let status = match data.get("_lease_ready_status") {
        Some(Value::String(s)) if matches!(s.as_str(), "pending" | "scheduled" | "retrying") => {
            s.clone()
        }
        // Queued legacy "failed" records are runnable retries, not terminal history.
        _ => "pending".into(),
    };
    ready.insert("status".into(), Value::String(status));
    for name in [
        "claim_token",
        "worker_id",
        "execution_phase",
        "lease_expires_at_ms",
        "_lease_ready_status",
    ] {
        ready.remove(name);
    }
    ready
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.detach();
    }
}

/// Read-only lease projection; legacy active work is never silently replayed.
pub(super) fn inspect(handle: &Value, data: &mut HashMap<String, Value>) -> Result<()> {
    inspect_snapshot(handle, data, true)
}
fn inspect_snapshot(
    handle: &Value,
    data: &mut HashMap<String, Value>,
    refresh: bool,
) -> Result<()> {
    let inflight = matches!(data.get("status"),Some(Value::String(s)) if matches!(s.as_str(),"active"|"claimed"));
    if !inflight {
        return Ok(());
    }
    let Some(Value::String(id)) = data.get("id") else {
        return Ok(());
    };
    let id = id.clone();
    if let Some(lease) = kv::job_leases::get(handle, &id)? {
        if !matches!(data.get("claim_token"),Some(Value::String(token)) if token==&lease.token) {
            if refresh {
                if let Value::Map(fresh) = kv::kv_get(handle, &format!("jobs:data:{id}"))? {
                    *data = fresh;
                    return inspect_snapshot(handle, data, false);
                }
            }
            return Err(IntentError::runtime_error(
                "job ownership changed during inspection; retry",
            ));
        }
        data.insert("worker_id".into(), Value::String(lease.owner));
        data.insert("claim_token".into(), Value::String(lease.token));
        data.insert("execution_phase".into(), Value::String(lease.phase));
        data.insert("lease_expires_at_ms".into(), Value::Int(lease.deadline_ms));
    } else {
        // Completion could have raced the first read. Prefer its newer state.
        if refresh {
            if let Value::Map(fresh) = kv::kv_get(handle, &format!("jobs:data:{id}"))? {
                *data = fresh;
                return inspect_snapshot(handle, data, false);
            }
            return Err(IntentError::runtime_error(
                "job disappeared during inspection",
            ));
        }
        if matches!(data.get("status"),Some(Value::String(s)) if matches!(s.as_str(),"active"|"claimed"))
        {
            let reason = if data.contains_key("claim_token") {
                "missing_lease"
            } else {
                "legacy_active_without_lease"
            };
            data.insert("status".into(), Value::String("outcome_unknown".into()));
            data.insert("recovery_reason".into(), Value::String(reason.into()));
        }
    }
    Ok(())
}
