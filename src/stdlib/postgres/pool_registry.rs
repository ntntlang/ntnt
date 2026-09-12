//! Process-local shared PostgreSQL pool ownership and admission.
//!
//! Keys are hashes only. Ready pools and pending verifications both consume one
//! slot. Same-key callers share verification; no network work runs under the
//! registry mutex. Every handle, operation and transaction snapshot must retain
//! an Arc<SharedPool>, never just clone its underlying deadpool Pool.
//!
//! LRU means last admission through a ready cache hit or completed creation
//! (coalesced creation callers share that admission). Queries do not refresh it.
//! Only a registry-sole-owner pool can be evicted. The warm-hit clone and eviction
//! decision are atomic under the same mutex; eviction explicitly closes deadpool.

use deadpool_postgres::Pool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(super) fn configured_capacity(
    value: Result<String, std::env::VarError>,
) -> Result<usize, &'static str> {
    const INVALID: &str = "Invalid NTNT_POSTGRES_MAX_SHARED_POOLS: expected a positive integer";
    match value {
        Err(std::env::VarError::NotPresent) => Ok(32),
        Ok(value) if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) => value
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or(INVALID),
        _ => Err(INVALID),
    }
}

pub(super) struct SharedPool(pub(super) Pool);

impl std::ops::Deref for SharedPool {
    type Target = Pool;
    fn deref(&self) -> &Pool {
        &self.0
    }
}

// Deliberately omit the deadpool configuration from diagnostics.
impl std::fmt::Debug for SharedPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedPool")
    }
}

type PoolResult = Result<Arc<SharedPool>, String>;

#[derive(Default)]
struct PendingPool {
    outcome: Mutex<Option<PoolResult>>,
    completed: std::sync::Condvar,
}

impl PendingPool {
    fn wait(&self) -> PoolResult {
        let mut outcome = self.outcome.lock().unwrap();
        while outcome.is_none() {
            outcome = self.completed.wait(outcome).unwrap();
        }
        outcome.as_ref().unwrap().clone()
    }
}

enum Entry {
    Ready(ReadyPool),
    Pending(Arc<PendingPool>),
}

struct ReadyPool {
    pool: Arc<SharedPool>,
    last_admitted: Instant,
}

pub(super) struct PoolRegistry {
    capacity: usize,
    entries: Mutex<HashMap<String, Entry>>,
}

impl PoolRegistry {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn get_or_create(
        &self,
        key: String,
        create: impl FnOnce() -> Result<Pool, String>,
    ) -> PoolResult {
        let pending = {
            let mut entries = self.entries.lock().unwrap();
            match entries.get_mut(&key) {
                Some(Entry::Ready(entry)) => {
                    entry.last_admitted = Instant::now();
                    return Ok(entry.pool.clone());
                }
                Some(Entry::Pending(pending)) => {
                    let pending = pending.clone();
                    drop(entries);
                    return pending.wait();
                }
                None => {}
            }
            if entries.len() >= self.capacity {
                let victim = entries
                    .iter()
                    .filter_map(|(key, entry)| {
                        let Entry::Ready(entry) = entry else {
                            return None;
                        };
                        (Arc::strong_count(&entry.pool) == 1).then_some((key, entry.last_admitted))
                    })
                    .min_by_key(|(_, last_admitted)| *last_admitted)
                    .map(|(key, _)| key.clone());
                let Some(victim) = victim else {
                    return Err("Postgres shared pool capacity reached (NTNT_POSTGRES_MAX_SHARED_POOLS); close unused handles and retry".into());
                };
                if let Some(Entry::Ready(entry)) = entries.remove(&victim) {
                    entry.pool.close();
                }
            }
            let pending = Arc::new(PendingPool::default());
            entries.insert(key.clone(), Entry::Pending(pending.clone()));
            pending
        };

        // Network work is outside the registry lock. The pending entry counts
        // against capacity and same-key callers wait for this exact outcome.
        let mut reservation = Reservation {
            registry: self,
            key,
            pending,
            completed: false,
        };
        let outcome = create().map(|pool| Arc::new(SharedPool(pool)));
        reservation.complete(outcome.clone());
        outcome
    }
}

/// A synchronous factory can unwind, but must never strand its slot or waiters.
/// No caller-supplied work runs under either mutex; recover poisoned locks here
/// so cleanup itself cannot double-panic during unwinding.
struct Reservation<'a> {
    registry: &'a PoolRegistry,
    key: String,
    pending: Arc<PendingPool>,
    completed: bool,
}

impl Reservation<'_> {
    fn complete(&mut self, outcome: PoolResult) {
        let mut entries = self
            .registry
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match &outcome {
            Ok(pool) => {
                entries.insert(
                    self.key.clone(),
                    Entry::Ready(ReadyPool {
                        pool: pool.clone(),
                        last_admitted: Instant::now(),
                    }),
                );
            }
            Err(_) => {
                entries.remove(&self.key);
            }
        }
        *self
            .pending
            .outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        self.completed = true;
        self.pending.completed.notify_all();
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.complete(Err(
                "Postgres shared pool creation interrupted; retry connect".into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unopened_pool() -> Result<Pool, String> {
        let mut config = deadpool_postgres::Config::new();
        config.host = Some("localhost".into());
        config.user = Some("postgres".into());
        config.dbname = Some("postgres".into());
        config
            .create_pool(
                Some(deadpool_postgres::Runtime::Tokio1),
                tokio_postgres::NoTls,
            )
            .map_err(|_| "test pool construction failed".into())
    }

    fn key(name: &str) -> String {
        super::super::hash_pool_key(name)
    }

    #[test]
    fn least_recently_admitted_unused_pool_is_closed() {
        let registry = PoolRegistry::new(2);
        let a = registry.get_or_create(key("a"), unopened_pool).unwrap();
        let a_observer = a.0.clone();
        drop(a);
        let b = registry.get_or_create(key("b"), unopened_pool).unwrap();
        let b_observer = b.0.clone();
        drop(b);
        // Refresh A's admission order without leaving a lease alive.
        drop(
            registry
                .get_or_create(key("a"), || panic!("warm reuse created a pool"))
                .unwrap(),
        );
        let c = registry.get_or_create(key("c"), unopened_pool);
        assert!(c.is_ok(), "unused pools must be evicted at capacity");
        assert!(
            b_observer.is_closed(),
            "eviction must explicitly close the LRU pool"
        );
        assert!(!a_observer.is_closed());
    }

    #[test]
    fn pending_distinct_creation_reserves_capacity_before_network_work() {
        let registry = Arc::new(PoolRegistry::new(1));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker_registry = registry.clone();
        let worker = std::thread::spawn(move || {
            worker_registry.get_or_create(key("slow"), || {
                started_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                unopened_pool()
            })
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let other = registry.get_or_create(key("other"), unopened_pool);
        release_tx.send(()).unwrap();
        let first = worker.join().unwrap().unwrap();
        assert!(
            other.is_err(),
            "pending creation must consume the only capacity slot"
        );
        assert!(!first.is_closed());
    }

    #[test]
    fn panicked_factory_releases_reservation_and_completes_waiters() {
        let registry = PoolRegistry::new(1);
        let mut pending = None;
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.get_or_create(key("panic"), || {
                let entries = registry.entries.lock().unwrap();
                if let Some(Entry::Pending(entry)) = entries.get(&key("panic")) {
                    pending = Some(entry.clone());
                }
                drop(entries);
                panic!("simulated factory unwind");
            })
        }));
        assert!(panic.is_err());
        assert!(
            registry.entries.lock().unwrap().is_empty(),
            "unwinding must free pending capacity"
        );
        assert!(
            matches!(&*pending.unwrap().outcome.lock().unwrap(), Some(Err(_))),
            "unwinding must publish an interruption outcome to waiters"
        );
        assert!(registry.get_or_create(key("retry"), unopened_pool).is_ok());
    }

    #[test]
    fn capacity_defaults_to_32_and_accepts_positive_integers_only() {
        assert_eq!(
            configured_capacity(Err(std::env::VarError::NotPresent)),
            Ok(32)
        );
        for value in ["1", "32", "64"] {
            assert_eq!(
                configured_capacity(Ok(value.into())),
                Ok(value.parse().unwrap())
            );
        }
        for value in [
            "0",
            "",
            "-1",
            "+2",
            " 2",
            "2 ",
            "1.5",
            "secret-canary",
            "9999999999999999999999999999999999999999",
        ] {
            let error = configured_capacity(Ok(value.into()));
            assert!(
                error.is_err(),
                "invalid shared-pool capacity must not silently default"
            );
            assert_eq!(
                error.unwrap_err(),
                "Invalid NTNT_POSTGRES_MAX_SHARED_POOLS: expected a positive integer"
            );
        }
        assert!(configured_capacity(Err(std::env::VarError::NotUnicode(
            std::ffi::OsString::from("secret-canary")
        )))
        .is_err());
    }

    #[derive(Clone, Copy)]
    enum FactoryOutcome {
        Success,
        Failure,
        Panic,
    }

    fn concurrent_same_key(outcome: FactoryOutcome) {
        use std::sync::mpsc;
        use std::time::Duration;
        const WAITERS: usize = 6;
        let timeout = Duration::from_secs(5);
        let registry = Arc::new(PoolRegistry::new(1));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let leader_registry = registry.clone();
        let leader = std::thread::spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                leader_registry.get_or_create(key("same"), || {
                    started_tx.send(()).unwrap();
                    release_rx.recv_timeout(timeout).unwrap();
                    match outcome {
                        FactoryOutcome::Success => unopened_pool(),
                        FactoryOutcome::Failure => Err("sanitized verification failure".into()),
                        FactoryOutcome::Panic => panic!("simulated factory unwind"),
                    }
                })
            }))
        });
        started_rx.recv_timeout(timeout).unwrap();
        let pending = {
            let entries = registry.entries.lock().unwrap();
            let Some(Entry::Pending(pending)) = entries.get(&key("same")) else {
                panic!("factory did not reserve its key");
            };
            pending.clone()
        };
        let (done_tx, done_rx) = mpsc::channel();
        let workers: Vec<_> = (0..WAITERS)
            .map(|_| {
                let registry = registry.clone();
                let done_tx = done_tx.clone();
                std::thread::spawn(move || {
                    let result =
                        registry.get_or_create(key("same"), || panic!("duplicate factory"));
                    done_tx.send(result).unwrap();
                })
            })
            .collect();
        let deadline = Instant::now() + timeout;
        // Registry, leader reservation, this observer, and all actual waiters.
        // Observe registration rather than sleeping or relying on scheduling.
        while Arc::strong_count(&pending) < 3 + WAITERS {
            assert!(
                Instant::now() < deadline,
                "same-key callers failed to join pending creation"
            );
            std::thread::yield_now();
        }
        release_tx.send(()).unwrap();
        let results: Vec<_> = (0..WAITERS)
            .map(|_| done_rx.recv_timeout(timeout).unwrap())
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let leader = leader.join().unwrap();
        match outcome {
            FactoryOutcome::Success => {
                let pool = leader.unwrap().unwrap();
                for result in &results {
                    assert!(Arc::ptr_eq(&pool, result.as_ref().unwrap()));
                }
                assert_eq!(registry.entries.lock().unwrap().len(), 1);
            }
            FactoryOutcome::Failure => {
                let expected = leader.unwrap().unwrap_err();
                assert!(results
                    .iter()
                    .all(|result| result.as_ref().unwrap_err() == &expected));
                assert!(registry.entries.lock().unwrap().is_empty());
            }
            FactoryOutcome::Panic => {
                assert!(leader.is_err());
                assert!(results.iter().all(|result| result.as_ref().unwrap_err()
                    == "Postgres shared pool creation interrupted; retry connect"));
                assert!(registry.entries.lock().unwrap().is_empty());
            }
        }
        drop(results);
        drop(pending);
        assert!(registry.get_or_create(key("same"), unopened_pool).is_ok());
    }

    #[test]
    fn concurrent_same_key_creates_exactly_one_verified_pool() {
        concurrent_same_key(FactoryOutcome::Success);
    }

    #[test]
    fn failed_creation_is_shared_with_waiters_and_retryable() {
        concurrent_same_key(FactoryOutcome::Failure);
    }

    #[test]
    fn panicked_creation_wakes_real_waiters_and_is_retryable() {
        concurrent_same_key(FactoryOutcome::Panic);
    }

    #[test]
    fn warm_hit_is_not_blocked_by_an_unrelated_factory() {
        use std::sync::mpsc;
        let timeout = std::time::Duration::from_secs(5);
        let registry = Arc::new(PoolRegistry::new(2));
        let warm = registry.get_or_create(key("warm"), unopened_pool).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let slow_registry = registry.clone();
        let slow = std::thread::spawn(move || {
            slow_registry.get_or_create(key("slow"), || {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(timeout).unwrap();
                unopened_pool()
            })
        });
        started_rx.recv_timeout(timeout).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let warm_worker = std::thread::spawn(move || {
            done_tx
                .send(registry.get_or_create(key("warm"), || panic!("warm factory ran")))
                .unwrap();
        });
        let warm_result = done_rx.recv_timeout(timeout);
        release_tx.send(()).unwrap();
        let slow_pool = slow.join().unwrap().unwrap();
        warm_worker.join().unwrap();
        let reused = warm_result
            .expect("warm hit blocked behind unrelated creation")
            .unwrap();
        assert!(Arc::ptr_eq(&warm, &reused));
        assert!(!slow_pool.is_closed());
    }

    #[test]
    fn eviction_skips_an_older_active_pool() {
        let registry = PoolRegistry::new(2);
        let active = registry
            .get_or_create(key("active"), unopened_pool)
            .unwrap();
        let idle = registry.get_or_create(key("idle"), unopened_pool).unwrap();
        let idle_observer = idle.0.clone();
        drop(idle);
        let new = registry.get_or_create(key("new"), unopened_pool).unwrap();
        assert!(idle_observer.is_closed());
        assert!(!active.is_closed());
        assert!(!new.is_closed());
    }

    #[test]
    fn active_lease_prevents_capacity_growth() {
        let registry = PoolRegistry::new(1);
        let first = registry.get_or_create(key("first"), unopened_pool).unwrap();
        let second = registry.get_or_create(key("second"), unopened_pool);
        assert!(
            second.is_err(),
            "active pool must not allow growth past the cap"
        );
        assert!(!first.is_closed());
    }
}
