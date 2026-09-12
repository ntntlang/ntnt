//! Compact process-local task history; never captures, graphs, error text or sync objects.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskKind {
    Spawn,
    After,
    Worker,
    Ephemeral,
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // Compact metadata retained for per-handle history; no global inspection API yet.
pub(super) struct TaskRecord {
    pub(super) id: u64,
    pub(super) state: TaskState,
    pub(super) outcome: TaskState,
    pub(super) kind: TaskKind,
    pub(super) created_at: Instant,
    pub(super) started_at: Option<Instant>,
    pub(super) completed_at: Instant,
    pub(super) retired_at: Instant,
    pub(super) duration: Option<Duration>,
}

// Conservative per-record charge for metadata, both B-tree indexes, node
// occupancy and allocator overhead. This estimates resident retained state,
// not process RSS or caller-owned graphs.
pub(super) const HISTORY_RECORD_BYTES: usize =
    3 * (std::mem::size_of::<(u64, TaskRecord)>() + std::mem::size_of::<(Instant, u64)>()) + 128;
// Small B-trees may reserve a root node with only one live record. Account for
// that fixed allowance separately rather than claiming per-record occupancy.
pub(super) const HISTORY_BASE_BYTES: usize =
    12 * (std::mem::size_of::<(u64, TaskRecord)>() + std::mem::size_of::<(Instant, u64)>()) + 256;

pub(super) struct RetentionPolicy {
    pub(super) result_ttl: Duration,
    pub(super) result_count: usize,
    pub(super) result_bytes: usize,
    pub(super) history_ttl: Duration,
    pub(super) history_count: usize,
    pub(super) history_bytes: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            result_ttl: Duration::from_secs(3600),
            result_count: 100_000,
            result_bytes: 128 * 1024 * 1024,
            history_ttl: task_removal_ttl(),
            history_count: 100_000,
            history_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Ordered indexes have exactly one node per retained record, never stale queue
/// entries from repeated polling. B-trees release nodes as records are removed;
/// they do not retain a hash table sized to historical peak throughput.
#[derive(Default)]
pub(super) struct TaskRegistry {
    pub(super) entries: BTreeMap<u64, TaskEntry>,
    pub(super) history: BTreeMap<u64, TaskRecord>,
    pub(super) results: BTreeSet<(Instant, u64)>,
    pub(super) result_bytes: usize,
    pub(super) history_order: BTreeSet<(Instant, u64)>,
}

impl TaskRegistry {
    pub(super) fn retire(
        &mut self,
        id: u64,
        state: TaskState,
        outcome: TaskState,
        completed_at: Instant,
        now: Instant,
        policy: &RetentionPolicy,
    ) {
        if let Some(entry) = self.entries.remove(&id) {
            if let Some(checked) = entry.last_checked_at {
                self.results.remove(&(checked, id));
                self.result_bytes -= entry.retained_bytes;
            }
            if entry.kind != TaskKind::Ephemeral {
                self.history_order.insert((now, id));
                self.history.insert(
                    id,
                    TaskRecord {
                        id,
                        state,
                        outcome,
                        kind: entry.kind,
                        created_at: entry.created_at,
                        started_at: entry.started_at,
                        completed_at,
                        retired_at: now,
                        duration: entry
                            .started_at
                            .map(|start| completed_at.saturating_duration_since(start)),
                    },
                );
            }
        }
        self.prune_history(now, policy);
    }

    fn history_bytes(&self) -> usize {
        if self.history.is_empty() {
            0
        } else {
            self.history
                .len()
                .saturating_mul(HISTORY_RECORD_BYTES)
                .saturating_add(HISTORY_BASE_BYTES)
        }
    }

    fn prune_history(&mut self, now: Instant, policy: &RetentionPolicy) {
        while let Some(&(at, id)) = self.history_order.first() {
            if self.history.len() <= policy.history_count
                && self.history_bytes() <= policy.history_bytes
                && now.saturating_duration_since(at) < policy.history_ttl
            {
                break;
            }
            self.history_order.pop_first();
            self.history.remove(&id);
        }
    }

    pub(super) fn enforce(&mut self, now: Instant, policy: &RetentionPolicy) {
        while let Some(&(at, id)) = self.results.first() {
            if self.results.len() <= policy.result_count
                && self.result_bytes <= policy.result_bytes
                && now.saturating_duration_since(at) < policy.result_ttl
            {
                break;
            }
            let inner_arc = Arc::clone(&self.entries[&id].inner);
            let mut inner = inner_arc.lock().unwrap_or_else(|e| e.into_inner());
            let outcome = inner.state;
            let completed = inner.completed_at.expect("indexed result is terminal");
            inner.state = TaskState::Expired;
            inner.result = None;
            inner.error_msg = None;
            self.retire(id, TaskState::Expired, outcome, completed, now, policy);
        }
        self.prune_history(now, policy);
    }

    pub(super) fn touch(&mut self, id: u64, now: Instant) {
        if let Some(entry) = self.entries.get_mut(&id) {
            if let Some(old) = entry.last_checked_at {
                let now = now.max(old); // An older observer may acquire the lock later.
                self.results.remove(&(old, id));
                entry.last_checked_at = Some(now);
                self.results.insert((now, id));
            }
        }
    }
}

/// Rolls back a registration if OS thread creation fails or the caller unwinds
/// before successful handoff. Running threads are finalized, never rolled back.
pub(crate) struct TaskStartGuard<'a> {
    runtime: &'a ConcurrencyRuntime,
    id: u64,
    committed: bool,
}

impl<'a> TaskStartGuard<'a> {
    pub(crate) fn new(runtime: &'a ConcurrencyRuntime, id: u64) -> Self {
        Self {
            runtime,
            id,
            committed: false,
        }
    }
    pub(crate) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for TaskStartGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.runtime.forget_task(self.id);
            self.runtime
                .active_tasks
                .fetch_sub(1, AtomicOrdering::Release);
        }
    }
}

pub(super) struct ScheduleStartGuard<'a> {
    runtime: &'a ConcurrencyRuntime,
    id: u64,
    committed: bool,
}

impl<'a> ScheduleStartGuard<'a> {
    pub(super) fn new(runtime: &'a ConcurrencyRuntime, id: u64) -> Self {
        Self {
            runtime,
            id,
            committed: false,
        }
    }
    pub(super) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ScheduleStartGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.runtime.cancel_schedule(self.id);
        }
    }
}

/// Owns the private children of one structured operation. Dropping the guard
/// does not wait for uncooperative losers; their execution counters remain active
/// until finalization, which discards any late result without reinsertion.
pub(super) struct StructuredTasks<'a> {
    pub(super) runtime: &'a ConcurrencyRuntime,
    pub(super) handles: Vec<Value>,
}

impl Drop for StructuredTasks<'_> {
    fn drop(&mut self) {
        for handle in &self.handles {
            if let Value::TaskHandle(id) = handle {
                self.runtime.forget_task(*id);
            }
        }
    }
}

impl SerializedValue {
    /// Estimated resident serialized storage, including inline enum slots and
    /// allocation capacity, not JSON length. Shared channel buffers, execution
    /// captures, allocator scratch and caller-owned return graphs are separate.
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(self.heap_bytes())
    }

    fn heap_bytes(&self) -> usize {
        fn array(values: &Vec<SerializedValue>) -> usize {
            values.iter().fold(
                values
                    .capacity()
                    .saturating_mul(std::mem::size_of::<SerializedValue>()),
                |bytes, value| bytes.saturating_add(value.heap_bytes()),
            )
        }
        fn map(fields: &HashMap<String, SerializedValue>) -> usize {
            // HashMap capacity excludes spare buckets; 2x covers bucket/control
            // storage plus the table's minimum allocation and alignment slack.
            let table = fields
                .capacity()
                .saturating_mul(2)
                .saturating_mul(std::mem::size_of::<(String, SerializedValue)>() + 1)
                .saturating_add(32);
            fields.iter().fold(table, |bytes, (key, value)| {
                bytes
                    .saturating_add(key.capacity())
                    .saturating_add(value.heap_bytes())
            })
        }
        match self {
            Self::String(value) => value.capacity(),
            Self::Array(values) => array(values),
            Self::Map(fields) => map(fields),
            Self::Struct { name, fields } => name.capacity().saturating_add(map(fields)),
            Self::EnumValue {
                enum_name,
                variant,
                values,
            } => enum_name
                .capacity()
                .saturating_add(variant.capacity())
                .saturating_add(array(values)),
            _ => 0,
        }
    }
}

impl ConcurrencyRuntime {
    pub(super) fn forget_task(&self, id: u64) {
        let cancelled = {
            let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            let Some(entry) = tasks.entries.remove(&id) else {
                return;
            };
            if let Some(at) = entry.last_checked_at {
                tasks.results.remove(&(at, id));
                tasks.result_bytes -= entry.retained_bytes;
            }
            let mut inner = entry.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.result = None;
            inner.error_msg = None;
            if inner.state != TaskState::Running {
                inner.state = TaskState::Expired;
            }
            entry.cancelled
        };
        cancelled.cancel();
    }

    /// Serialize outside registry locks; callers may return arbitrary graphs or
    /// errors. Publish and enforce atomically before notifying waiting callers.
    pub(super) fn finish_task(
        &self,
        id: u64,
        result: std::result::Result<Result<Value>, Box<dyn std::any::Any + Send>>,
        inner_arc: &Arc<Mutex<TaskInner>>,
        notify: &Arc<(Mutex<bool>, Condvar)>,
        now: Instant,
    ) {
        let mut completion = TaskInner {
            state: TaskState::Completed,
            result: None,
            error_msg: None,
            completed_at: Some(now),
        };
        match result {
            Ok(Ok(value)) => match SerializedValue::from_value(&value) {
                Ok(serialized) => completion.result = Some(serialized),
                Err(_) => {
                    completion.state = TaskState::Failed;
                    completion.error_msg = Some(format!("Task returned a non-serializable value ({}). Only Int, Float, Bool, String, Array, Map, Struct, Enum can cross task boundaries.", value.type_name()));
                }
            },
            Ok(Err(e)) => {
                completion.state = TaskState::Failed;
                completion.error_msg = Some(e.to_string());
            }
            Err(panic) => {
                completion.state = TaskState::Panicked;
                completion.error_msg = Some(if let Some(s) = panic.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "Task panicked".to_string()
                });
            }
        }
        let bytes = completion
            .result
            .as_ref()
            .map_or(0, SerializedValue::retained_bytes)
            .saturating_add(completion.error_msg.as_ref().map_or(0, String::capacity));
        {
            let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            let mut inner = inner_arc.lock().unwrap_or_else(|e| e.into_inner());
            if inner.state != TaskState::Running {
                return;
            }
            if let Some(entry) = tasks.entries.get_mut(&id) {
                *inner = completion;
                if entry.kind != TaskKind::Ephemeral {
                    entry.last_checked_at = Some(now);
                    entry.retained_bytes = bytes;
                    tasks.result_bytes = tasks.result_bytes.saturating_add(bytes);
                    tasks.results.insert((now, id));
                }
            } else {
                // Forgotten structured child: never resurrect registry ownership.
                inner.state = TaskState::Expired;
                inner.completed_at = Some(now);
                inner.result = None;
                inner.error_msg = None;
            }
            drop(inner);
            tasks.enforce(now, &self.retention);
        }
        self.active_tasks.fetch_sub(1, AtomicOrdering::Release);
        let mut done = notify.0.lock().unwrap_or_else(|e| e.into_inner());
        *done = true;
        notify.1.notify_all();
    }
}
