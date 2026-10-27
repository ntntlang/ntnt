//! Built-in HTTP rate limiting (#294).
//!
//! Off unless `NTNT_RATE_LIMIT=on`, in every environment. When off, the request
//! path pays one branch. When on, each dynamic-route request is checked against
//! a per-client budget before the body is read or the interpreter is involved.
//!
//! Algorithm: GCRA. One `u64` per key holds the theoretical arrival time (TAT)
//! in nanoseconds from a process-local monotonic base. The lowest bit of the
//! stored value flags "trip already logged" for the current limited episode.
//!
//! Store: a fixed number of shards, each a `Mutex<HashMap<u64, u64>>` keyed by a
//! keyed hash of the limiter key. Capacity is bounded; a full shard first drops
//! expired entries, then accounts new keys to a shared overflow key instead of
//! evicting live ones. No allocation for existing keys, no `.await` under a lock,
//! and a poisoned shard is recovered and cleared rather than panicking.

use crate::error::{IntentError, Result};
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const SHARDS: usize = 64;
/// Overflow budgets live in a reserved key range at the top of u64. Each
/// (class, rate, burst) gets its own slot, so a full shard never makes one
/// limit read another limit's saved time.
const OVERFLOW_SLOTS: u64 = 4096;
const OVERFLOW_BASE: u64 = u64::MAX - OVERFLOW_SLOTS + 1;
const OVERFLOW_WRITE: u64 = 0;
const OVERFLOW_READ: u64 = 1;

fn overflow_slot(class_tag: u64, rate: RateSpec, burst: u64) -> u64 {
    // Deterministic mix (not keyed: rates are configuration, not attacker input).
    let mut x = class_tag
        ^ rate.count.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ rate.period_ns.rotate_left(17)
        ^ burst.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    OVERFLOW_BASE + (x % OVERFLOW_SLOTS)
}
/// A full shard is swept at most this often (bounds scan cost under key churn).
const SWEEP_INTERVAL_NS: u64 = 1_000_000_000;
const LOGGED_BIT: u64 = 1;

/// A rate such as `30/minute`: `count` requests per `period_ns`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateSpec {
    pub count: u64,
    pub period_ns: u64,
}

impl RateSpec {
    /// Parse `N/second|minute|hour|day`.
    pub fn parse(text: &str) -> std::result::Result<RateSpec, String> {
        let bad = || format!("invalid rate '{text}' (expected N/second|minute|hour|day)");
        let (count, unit) = text.trim().split_once('/').ok_or_else(bad)?;
        let count: u64 = count.trim().parse().map_err(|_| bad())?;
        if count == 0 || count > 1_000_000_000 {
            return Err(bad());
        }
        let secs: u64 = match unit.trim() {
            "second" => 1,
            "minute" => 60,
            "hour" => 3_600,
            "day" => 86_400,
            _ => return Err(bad()),
        };
        Ok(RateSpec {
            count,
            period_ns: secs * 1_000_000_000,
        })
    }

    /// Nanoseconds between requests at the sustained rate (at least 2 so the
    /// flag bit never swallows the interval).
    fn interval_ns(&self) -> u64 {
        (self.period_ns / self.count).max(2)
    }
}

/// A per-route override: a rate, or `off` to exempt the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteRate {
    Off,
    Rate(RateSpec),
}

impl RouteRate {
    pub fn parse(text: &str) -> std::result::Result<RouteRate, String> {
        if text.trim().eq_ignore_ascii_case("off") {
            return Ok(RouteRate::Off);
        }
        RateSpec::parse(text).map(RouteRate::Rate)
    }
}

/// Outcome of one check, with what the 429 headers need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    /// Seconds until the next request would be allowed (0 when allowed).
    pub retry_after_secs: u64,
    /// Seconds until the budget is fully restored.
    pub reset_secs: u64,
    /// True only for the first denial of a limited episode (log it once).
    pub first_trip: bool,
}

fn ceil_secs(ns: u64) -> u64 {
    ns.div_ceil(1_000_000_000)
}

/// GCRA step for one key. `stored` is the previous value (TAT | flag), `now`
/// is nanoseconds from the base. Returns the decision and the value to store.
fn gcra(stored: Option<u64>, now: u64, rate: RateSpec, burst: u64) -> (Decision, u64) {
    let t = rate.interval_ns();
    let tau = t.saturating_mul(burst.max(1));
    let (tat, logged) = match stored {
        Some(v) => (v & !LOGGED_BIT, v & LOGGED_BIT != 0),
        None => (now, false),
    };
    let base = tat.max(now);
    let new_tat = base.saturating_add(t);
    let ahead = new_tat - now; // how far the client is ahead of schedule
    if ahead <= tau {
        let remaining = (tau - ahead) / t;
        let decision = Decision {
            allowed: true,
            limit: burst,
            remaining,
            retry_after_secs: 0,
            reset_secs: ceil_secs(ahead),
            first_trip: false,
        };
        (decision, new_tat & !LOGGED_BIT)
    } else {
        let decision = Decision {
            allowed: false,
            limit: burst,
            remaining: 0,
            retry_after_secs: ceil_secs(ahead - tau).max(1),
            reset_secs: ceil_secs(tat.saturating_sub(now)),
            first_trip: !logged,
        };
        // Denials do not advance the TAT; only the logged flag is recorded.
        (decision, tat | LOGGED_BIT)
    }
}

struct Shard {
    map: HashMap<u64, u64>,
    /// Monotonic ns of the last full-shard sweep (u64::MAX = never).
    last_sweep: u64,
}

/// Bounded, sharded GCRA store with an injectable clock (nanoseconds).
pub struct Store {
    shards: Vec<Mutex<Shard>>,
    per_shard: usize,
    hasher: RandomState,
}

impl Store {
    pub fn new(max_keys: usize) -> Store {
        let per_shard = (max_keys / SHARDS).max(1);
        Store {
            shards: (0..SHARDS)
                .map(|_| {
                    Mutex::new(Shard {
                        map: HashMap::with_capacity(per_shard.min(1024)),
                        last_sweep: u64::MAX,
                    })
                })
                .collect(),
            per_shard,
            hasher: RandomState::new(),
        }
    }

    /// Keyed hash, so an attacker cannot aim keys at one shard or at the
    /// reserved overflow values.
    pub fn key<K: Hash>(&self, key: K) -> u64 {
        let mut h = self.hasher.build_hasher();
        key.hash(&mut h);
        h.finish().min(OVERFLOW_BASE - 1)
    }

    /// Check `key` at `now`. `overflow` is the class tag of the shared budget
    /// used when the shard is full; the slot is per (class, rate, burst).
    pub fn check(&self, key: u64, overflow: u64, rate: RateSpec, burst: u64, now: u64) -> Decision {
        let shard = &self.shards[(key as usize) & (SHARDS - 1)];
        let mut guard = match shard.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                // Never panic or block the server: recover and start the shard fresh.
                let mut guard = poisoned.into_inner();
                guard.map.clear();
                guard.last_sweep = u64::MAX;
                shard.clear_poison();
                guard
            }
        };
        let shard = &mut *guard;
        let mut slot = key;
        if !shard.map.contains_key(&key) && shard.map.len() >= self.per_shard {
            let due = shard.last_sweep == u64::MAX
                || now.saturating_sub(shard.last_sweep) >= SWEEP_INTERVAL_NS;
            if due {
                // Entries whose TAT has passed (overflow included) hold no state.
                shard.map.retain(|_, v| (*v & !LOGGED_BIT) > now);
                shard.last_sweep = now;
            }
            if shard.map.len() >= self.per_shard {
                // Still full: new keys share the class's overflow budget; live
                // keys are never evicted.
                slot = overflow_slot(overflow, rate, burst);
            }
        }
        let stored = shard.map.get(&slot).copied();
        let (decision, value) = gcra(stored, now, rate, burst);
        shard.map.insert(slot, value);
        decision
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|s| s.lock().unwrap().map.len())
            .sum()
    }
}

/// Request class for the built-in limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Write,
    Read,
}

/// Parsed `NTNT_RATE_LIMIT*` configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub enabled: bool,
    pub write: RateSpec,
    pub read: RateSpec,
    pub write_burst: u64,
    pub read_burst: u64,
    pub max_keys: usize,
    pub exempt: Vec<String>,
    pub ipv6_prefix: u8,
    pub headers_on_allowed: bool,
}

impl Config {
    /// Build from a lookup (env in production, a map in tests).
    pub fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
    ) -> std::result::Result<Config, String> {
        let switch = get("NTNT_RATE_LIMIT").unwrap_or_default();
        let enabled = match switch.trim().to_ascii_lowercase().as_str() {
            "" | "off" | "0" | "false" | "no" => false,
            "on" | "1" | "true" | "yes" => true,
            other => return Err(format!("invalid NTNT_RATE_LIMIT '{other}' (use on or off)")),
        };
        let rate = |name: &str, default: &str| -> std::result::Result<RateSpec, String> {
            RateSpec::parse(&get(name).unwrap_or_else(|| default.to_string()))
                .map_err(|e| format!("{name}: {e}"))
        };
        let write = rate("NTNT_RATE_LIMIT_WRITE", "30/minute")?;
        let read = rate("NTNT_RATE_LIMIT_READ", "600/minute")?;
        let burst = match get("NTNT_RATE_LIMIT_BURST") {
            None => None,
            Some(v) => Some(
                v.trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|b| *b >= 1 && *b <= 1_000_000)
                    .ok_or_else(|| format!("invalid NTNT_RATE_LIMIT_BURST '{v}'"))?,
            ),
        };
        let max_keys = match get("NTNT_RATE_LIMIT_MAX_KEYS") {
            None => 100_000,
            Some(v) => v
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= SHARDS && *n <= 100_000_000)
                .ok_or_else(|| format!("invalid NTNT_RATE_LIMIT_MAX_KEYS '{v}' (min {SHARDS})"))?,
        };
        let exempt = get("NTNT_RATE_LIMIT_EXEMPT")
            .unwrap_or_else(|| "/health,/ready".to_string())
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let ipv6_prefix = match get("NTNT_RATE_LIMIT_IPV6_PREFIX") {
            None => 64,
            Some(v) => v
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|p| *p >= 1 && *p <= 128)
                .ok_or_else(|| format!("invalid NTNT_RATE_LIMIT_IPV6_PREFIX '{v}'"))?,
        };
        let headers_on_allowed = match get("NTNT_RATE_LIMIT_HEADERS").as_deref().map(str::trim) {
            None | Some("") => false,
            Some("all") => true,
            Some(other) => {
                return Err(format!(
                    "invalid NTNT_RATE_LIMIT_HEADERS '{other}' (use all, or leave unset)"
                ))
            }
        };
        Ok(Config {
            enabled,
            write,
            read,
            write_burst: burst.unwrap_or(write.count),
            read_burst: burst.unwrap_or(read.count),
            max_keys,
            exempt,
            ipv6_prefix,
            headers_on_allowed,
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "write {}/{}s (burst {}), read {}/{}s (burst {}), max keys {}, exempt {:?}",
            self.write.count,
            self.write.period_ns / 1_000_000_000,
            self.write_burst,
            self.read.count,
            self.read.period_ns / 1_000_000_000,
            self.read_burst,
            self.max_keys,
            self.exempt
        )
    }
}

/// Classify a request; `None` means never limited (OPTIONS and unusual methods
/// are treated as writes except OPTIONS, which is preflight).
pub fn classify(method: &str) -> Option<Class> {
    match method {
        "GET" | "HEAD" => Some(Class::Read),
        "OPTIONS" => None,
        _ => Some(Class::Write),
    }
}

/// Client key: IPv4 exact, IPv6 truncated to the configured prefix. The
/// family is part of the key, so the two can never collide.
pub fn client_key(ip: IpAddr, ipv6_prefix: u8) -> (u8, u128) {
    match ip {
        IpAddr::V4(v4) => (4, u32::from(v4) as u128),
        IpAddr::V6(v6) => {
            let mask = if ipv6_prefix >= 128 {
                u128::MAX
            } else {
                !(u128::MAX >> ipv6_prefix)
            };
            (6, u128::from(v6) & mask)
        }
    }
}

/// The process limiter: configuration plus store.
pub struct Limiter {
    pub config: Config,
    store: Store,
    base: Instant,
}

impl Limiter {
    pub fn new(config: Config) -> Limiter {
        let store = Store::new(config.max_keys);
        Limiter {
            config,
            store,
            base: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.base.elapsed().as_nanos().min(u64::MAX as u128 - 1) as u64
    }

    /// Check one request. `None` when the request is not subject to limits.
    pub fn check_request(&self, method: &str, path: &str, client: IpAddr) -> Option<Decision> {
        self.check_route(method, path, client, None)
    }

    /// Check one request against its route. A route rate replaces the global
    /// class budget and is counted per (route, client); `off` exempts it.
    pub fn check_route(
        &self,
        method: &str,
        path: &str,
        client: IpAddr,
        route: Option<(&str, RouteRate)>,
    ) -> Option<Decision> {
        let class = classify(method)?;
        if self.config.exempt.iter().any(|p| p == path) {
            return None;
        }
        if let Some((route_id, override_rate)) = route {
            let RouteRate::Rate(rate) = override_rate else {
                return None;
            };
            let key = self.store.key((
                1u8,
                route_id,
                class,
                client_key(client, self.config.ipv6_prefix),
            ));
            let overflow = match class {
                Class::Write => OVERFLOW_WRITE,
                Class::Read => OVERFLOW_READ,
            };
            return Some(
                self.store
                    .check(key, overflow, rate, rate.count, self.now()),
            );
        }
        let (rate, burst, overflow) = match class {
            Class::Write => (self.config.write, self.config.write_burst, OVERFLOW_WRITE),
            Class::Read => (self.config.read, self.config.read_burst, OVERFLOW_READ),
        };
        let key = self
            .store
            .key((0u8, class, client_key(client, self.config.ipv6_prefix)));
        Some(self.store.check(key, overflow, rate, burst, self.now()))
    }
}

static LIMITER: OnceLock<Option<Limiter>> = OnceLock::new();

/// Store behind `check_limit`: always available (an explicit call), separate
/// from the built-in limiter, bounded by `NTNT_RATE_LIMIT_MAX_KEYS`.
struct KeyLimits {
    store: Store,
    base: Instant,
}

static KEY_LIMITS: OnceLock<KeyLimits> = OnceLock::new();

fn key_limits() -> &'static KeyLimits {
    KEY_LIMITS.get_or_init(|| {
        let max_keys = std::env::var("NTNT_RATE_LIMIT_MAX_KEYS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n >= SHARDS && *n <= 100_000_000)
            .unwrap_or(100_000);
        KeyLimits {
            store: Store::new(max_keys),
            base: Instant::now(),
        }
    })
}

/// `check_limit(key, rate)`: an app-defined key (email, account, ...) in this
/// process. The key is hashed with a process-random key; it is never stored.
pub fn check_key(key: &str, rate: RateSpec) -> Decision {
    let limits = key_limits();
    let now = limits.base.elapsed().as_nanos().min(u64::MAX as u128 - 1) as u64;
    let hashed = limits.store.key((2u8, key));
    limits
        .store
        .check(hashed, OVERFLOW_WRITE, rate, rate.count, now)
}

/// Parse configuration once (from `listen()`); invalid values fail startup.
/// Returns the limiter when enabled.
pub fn init_from_env() -> Result<Option<&'static Limiter>> {
    if let Some(existing) = LIMITER.get() {
        return Ok(existing.as_ref());
    }
    let config =
        Config::from_lookup(|k| std::env::var(k).ok()).map_err(IntentError::runtime_error)?;
    let limiter = if config.enabled {
        Some(Limiter::new(config))
    } else {
        None
    };
    let _ = LIMITER.set(limiter);
    Ok(LIMITER.get().and_then(|l| l.as_ref()))
}

/// The enabled limiter, if any (None before `init_from_env` or when off).
#[inline]
pub fn limiter() -> Option<&'static Limiter> {
    LIMITER.get().and_then(|l| l.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000_000;

    fn cfg(pairs: &[(&str, &str)]) -> std::result::Result<Config, String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| owned.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
    }

    #[test]
    fn default_is_off_and_defaults_parse() {
        let c = cfg(&[]).unwrap();
        assert!(!c.enabled);
        assert_eq!(c.write, RateSpec::parse("30/minute").unwrap());
        assert_eq!(c.read_burst, 600);
        assert_eq!(c.exempt, vec!["/health", "/ready"]);
        assert_eq!(c.ipv6_prefix, 64);
        assert!(cfg(&[("NTNT_RATE_LIMIT", "off")]).unwrap().enabled == false);
        assert!(cfg(&[("NTNT_RATE_LIMIT", "on")]).unwrap().enabled);
    }

    #[test]
    fn invalid_configuration_fails() {
        for (k, v) in [
            ("NTNT_RATE_LIMIT", "maybe"),
            ("NTNT_RATE_LIMIT_WRITE", "0/minute"),
            ("NTNT_RATE_LIMIT_WRITE", "5/fortnight"),
            ("NTNT_RATE_LIMIT_READ", "lots"),
            ("NTNT_RATE_LIMIT_BURST", "0"),
            ("NTNT_RATE_LIMIT_MAX_KEYS", "10"),
            ("NTNT_RATE_LIMIT_IPV6_PREFIX", "0"),
            ("NTNT_RATE_LIMIT_HEADERS", "al"),
        ] {
            assert!(cfg(&[(k, v)]).is_err(), "{k}={v}");
        }
        assert_eq!(RateSpec::parse("2/day").unwrap().period_ns, 86_400 * S);
    }

    #[test]
    fn gcra_allows_burst_then_denies_then_recovers() {
        let store = Store::new(1_000);
        let rate = RateSpec::parse("3/minute").unwrap(); // one per 20 s
        let k = store.key("a");
        let t0 = 10 * S;
        for i in 0..3 {
            let d = store.check(k, OVERFLOW_WRITE, rate, 3, t0);
            assert!(d.allowed, "request {i}");
            assert_eq!(d.remaining, 2 - i);
        }
        let denied = store.check(k, OVERFLOW_WRITE, rate, 3, t0);
        assert!(!denied.allowed);
        assert!(denied.first_trip);
        assert_eq!(denied.retry_after_secs, 20);
        let again = store.check(k, OVERFLOW_WRITE, rate, 3, t0 + S);
        assert!(!again.allowed && !again.first_trip, "trip logged once");
        assert_eq!(again.retry_after_secs, 19);
        // One interval later exactly one more request fits.
        assert!(store.check(k, OVERFLOW_WRITE, rate, 3, t0 + 20 * S).allowed);
        assert!(!store.check(k, OVERFLOW_WRITE, rate, 3, t0 + 20 * S).allowed);
        // After the full period the budget is back.
        let later = store.check(k, OVERFLOW_WRITE, rate, 3, t0 + 200 * S);
        assert!(later.allowed);
        assert_eq!(later.remaining, 2);
    }

    #[test]
    fn keys_are_independent() {
        let store = Store::new(1_000);
        let rate = RateSpec::parse("1/minute").unwrap();
        assert!(
            store
                .check(store.key("a"), OVERFLOW_WRITE, rate, 1, S)
                .allowed
        );
        assert!(
            !store
                .check(store.key("a"), OVERFLOW_WRITE, rate, 1, S)
                .allowed
        );
        assert!(
            store
                .check(store.key("b"), OVERFLOW_WRITE, rate, 1, S)
                .allowed
        );
    }

    /// Find `n` keys landing in shard 0.
    fn shard0_keys(store: &Store, tag: &str, n: usize) -> Vec<u64> {
        let mut out = Vec::new();
        let mut i = 0u64;
        while out.len() < n {
            let k = store.key((tag, i));
            i += 1;
            if (k as usize) & (SHARDS - 1) == 0 {
                out.push(k);
            }
        }
        out
    }

    #[test]
    fn full_shard_uses_overflow_without_evicting_live_keys_then_recovers() {
        let store = Store::new(SHARDS); // one slot per shard
        let rate = RateSpec::parse("1/minute").unwrap();
        let now = 100 * S;
        let keys = shard0_keys(&store, "k", 3);
        let (live, sprayed, later_client) = (keys[0], keys[1], keys[2]);
        assert!(store.check(live, OVERFLOW_WRITE, rate, 1, now).allowed);
        // Shard full of a live key: the new key lands on the overflow budget.
        assert!(store.check(sprayed, OVERFLOW_WRITE, rate, 1, now).allowed);
        assert!(!store.check(sprayed, OVERFLOW_WRITE, rate, 1, now).allowed);
        assert!(
            !store.check(live, OVERFLOW_WRITE, rate, 1, now).allowed,
            "live key kept its state"
        );
        assert!(store.len() <= SHARDS + 2);
        // After everything (overflow included) expires, a new client gets its
        // own slot again instead of being stuck on the shared budget.
        let later = now + 600 * S;
        assert!(
            store
                .check(later_client, OVERFLOW_WRITE, rate, 1, later)
                .allowed
        );
        assert!(
            !store
                .check(later_client, OVERFLOW_WRITE, rate, 1, later)
                .allowed
        );
        assert!(
            store
                .check(sprayed, OVERFLOW_WRITE, rate, 1, later + 1)
                .allowed,
            "overflow not shared with later_client"
        );
    }

    #[test]
    fn overflow_budgets_are_separate_per_class() {
        let store = Store::new(SHARDS);
        let write = RateSpec::parse("1/minute").unwrap();
        let read = RateSpec::parse("60/minute").unwrap();
        let now = 100 * S;
        let keys = shard0_keys(&store, "c", 3);
        assert!(store.check(keys[0], OVERFLOW_WRITE, write, 1, now).allowed);
        assert!(store.check(keys[1], OVERFLOW_WRITE, write, 1, now).allowed); // write overflow spent
        assert!(
            store.check(keys[2], OVERFLOW_READ, read, 60, now).allowed,
            "read overflow untouched"
        );
    }

    #[test]
    fn overflow_budgets_are_separate_per_rate() {
        let store = Store::new(SHARDS);
        let slow = RateSpec::parse("1/day").unwrap();
        let fast = RateSpec::parse("100/minute").unwrap();
        let now = 100 * S;
        let keys = shard0_keys(&store, "r", 3);
        assert!(store.check(keys[0], OVERFLOW_WRITE, slow, 1, now).allowed);
        // keys[1] lands on the 1/day overflow and spends it.
        assert!(store.check(keys[1], OVERFLOW_WRITE, slow, 1, now).allowed);
        // A different rate in the same full shard has its own overflow budget.
        assert!(store.check(keys[2], OVERFLOW_WRITE, fast, 100, now).allowed);
        assert_ne!(
            overflow_slot(OVERFLOW_WRITE, slow, 1),
            overflow_slot(OVERFLOW_WRITE, fast, 100)
        );
    }

    #[test]
    fn full_shard_sweeps_at_most_once_per_interval() {
        let store = Store::new(SHARDS); // one slot per shard
        let fast = RateSpec::parse("1000000/second").unwrap(); // 1 µs interval
        let now = 100 * S;
        let keys = shard0_keys(&store, "w", 3);
        assert!(store.check(keys[0], OVERFLOW_WRITE, fast, 1, now).allowed); // TAT now+1µs
                                                                             // First sweep (never swept before) reclaims keys[0]; keys[1] takes the slot.
        assert!(
            store
                .check(keys[1], OVERFLOW_WRITE, fast, 1, now + 2_000)
                .allowed
        );
        assert!(store.shards[0].lock().unwrap().map.contains_key(&keys[1]));
        // keys[1] has expired, but the last sweep was 3 µs ago: no scan, overflow.
        assert!(
            store
                .check(keys[2], OVERFLOW_WRITE, fast, 1, now + 5_000)
                .allowed
        );
        {
            let shard = store.shards[0].lock().unwrap();
            assert!(
                shard
                    .map
                    .contains_key(&overflow_slot(OVERFLOW_WRITE, fast, 1)),
                "no sweep within the interval"
            );
            assert!(
                shard.map.contains_key(&keys[1]),
                "expired key not yet swept"
            );
        }
        // One interval later the sweep runs and keys[2] gets its own slot.
        assert!(
            store
                .check(keys[2], OVERFLOW_WRITE, fast, 1, now + 2_000 + S + 1)
                .allowed
        );
        let shard = store.shards[0].lock().unwrap();
        assert!(shard.map.contains_key(&keys[2]));
        assert!(!shard
            .map
            .contains_key(&overflow_slot(OVERFLOW_WRITE, fast, 1)));
    }

    #[test]
    fn ipv6_is_grouped_by_prefix_and_never_collides_with_ipv4() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::9".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(client_key(a, 64), client_key(b, 64));
        assert_ne!(client_key(a, 64), client_key(c, 64));
        assert_ne!(client_key(a, 128), client_key(b, 128));
        let v4: IpAddr = "0.0.0.1".parse().unwrap();
        let v6: IpAddr = "::1".parse().unwrap();
        assert_ne!(client_key(v4, 128), client_key(v6, 128));
    }

    #[test]
    fn limiter_classes_exemptions_and_options() {
        let c = cfg(&[
            ("NTNT_RATE_LIMIT", "on"),
            ("NTNT_RATE_LIMIT_WRITE", "2/minute"),
            ("NTNT_RATE_LIMIT_READ", "3/minute"),
        ])
        .unwrap();
        let l = Limiter::new(c);
        let ip: IpAddr = "198.51.100.7".parse().unwrap();
        let other: IpAddr = "198.51.100.8".parse().unwrap();
        assert!(l.check_request("POST", "/contact", ip).unwrap().allowed);
        assert!(l.check_request("POST", "/contact", ip).unwrap().allowed);
        assert!(!l.check_request("PUT", "/contact", ip).unwrap().allowed);
        assert!(l.check_request("POST", "/contact", other).unwrap().allowed);
        // Reads have their own budget; exempt paths and preflight are unlimited.
        assert!(l.check_request("GET", "/", ip).unwrap().allowed);
        assert!(l.check_request("POST", "/health", ip).is_none());
        assert!(l.check_request("OPTIONS", "/contact", ip).is_none());
    }

    #[test]
    fn route_rates_override_and_exempt() {
        let c = cfg(&[
            ("NTNT_RATE_LIMIT", "on"),
            ("NTNT_RATE_LIMIT_WRITE", "100/minute"),
        ])
        .unwrap();
        let l = Limiter::new(c);
        let ip: IpAddr = "198.51.100.7".parse().unwrap();
        let contact = Some(("POST /contact", RouteRate::parse("2/minute").unwrap()));
        assert!(
            l.check_route("POST", "/contact", ip, contact)
                .unwrap()
                .allowed
        );
        assert!(
            l.check_route("POST", "/contact", ip, contact)
                .unwrap()
                .allowed
        );
        assert!(
            !l.check_route("POST", "/contact", ip, contact)
                .unwrap()
                .allowed
        );
        // The route budget is separate from the global write budget.
        assert!(l.check_route("POST", "/other", ip, None).unwrap().allowed);
        let off = Some(("POST /hook", RouteRate::parse("off").unwrap()));
        assert!(l.check_route("POST", "/hook", ip, off).is_none());
        assert!(RouteRate::parse("5/fortnight").is_err());
    }

    #[test]
    fn check_key_is_per_key_and_bounded_by_rate() {
        let rate = RateSpec::parse("2/minute").unwrap();
        let a = format!("test-a-{}", std::process::id());
        assert!(check_key(&a, rate).allowed);
        let d = check_key(&a, rate);
        assert!(d.allowed && d.remaining == 0);
        let denied = check_key(&a, rate);
        assert!(!denied.allowed && denied.retry_after_secs >= 29);
        assert!(check_key(&format!("{a}-other"), rate).allowed);
    }

    #[test]
    fn poisoned_shard_recovers() {
        let store = std::sync::Arc::new(Store::new(1_000));
        let rate = RateSpec::parse("1/minute").unwrap();
        let k = store.key("p");
        let shard = (k as usize) & (SHARDS - 1);
        let s2 = store.clone();
        let _ = std::thread::spawn(move || {
            let _g = s2.shards[shard].lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(store.shards[shard].is_poisoned());
        assert!(store.check(k, OVERFLOW_WRITE, rate, 1, S).allowed);
        assert!(!store.shards[shard].is_poisoned());
    }
}
