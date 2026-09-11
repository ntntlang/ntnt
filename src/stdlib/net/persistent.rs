//! Persistent ICMP ownership. Registry locks never cover network I/O.
//! Sends and short, kernel-woken receives occur under the owner lock. Close
//! signals cancellation before taking that lock, then drops the sole descriptor
//! without racing a syscall using a recycled fd.
use super::icmp::{self, EchoSocket, IcmpProbeEvent};
use crate::interpreter::Value;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

type R<T> = Result<T, String>;
const LIMIT: usize = 128;
const SWEEP: Duration = Duration::from_millis(100);
const POLL: Duration = Duration::from_millis(5);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static IDS: AtomicU64 = AtomicU64::new(1);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static REGISTRY: OnceLock<Mutex<Vec<Weak<Owner>>>> = OnceLock::new();
static REAPER: OnceLock<R<()>> = OnceLock::new();
thread_local! { static CONTEXT: Cell<u64> = const { Cell::new(0) }; }
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // All protected mutations maintain valid states even on unwind. Cleanup
    // must still dispose of the socket if an unrelated Rust panic poisoned it.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
fn registry() -> &'static Mutex<Vec<Weak<Owner>>> {
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Owned by the interpreter itself, never by its cyclic lexical environments.
#[derive(Debug)]
pub(crate) struct Scope(u64);
impl Scope {
    pub(crate) fn new() -> Self {
        Self(IDS.fetch_add(1, Ordering::Relaxed))
    }
    pub(crate) fn enter(&self) -> ContextGuard {
        ContextGuard(CONTEXT.with(|c| c.replace(self.0)))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        for owner in snapshot() {
            if owner.context == self.0 {
                owner.terminate(Status::Closed);
            }
        }
    }
}
pub(crate) struct ContextGuard(u64);
impl Drop for ContextGuard {
    fn drop(&mut self) {
        CONTEXT.with(|c| c.set(self.0));
    }
}
struct Permit;
impl Permit {
    fn reserve() -> R<Self> {
        LIVE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < LIMIT).then_some(n + 1)
        })
        .map_err(|_| "capacity: persistent ICMP socket limit (128)".to_string())?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Status {
    Open,
    Closed,
    Expired,
}
struct Resource {
    socket: EchoSocket,
    _permit: Permit,
}
struct State {
    resource: Option<Resource>,
    status: Status,
    active: bool,
    deadline: Instant,
    logical: u64,
}
pub struct Owner {
    state: Mutex<State>,
    closing: AtomicBool,
    context: u64,
    nonce: [u8; 16],
    idle: Duration,
}
impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<ProbeHandle>")
    }
}
impl Owner {
    fn terminate(&self, status: Status) {
        // Stop before taking the lock so a busy receiver cannot repeatedly
        // start new receive slices ahead of the waiting closer.
        self.closing.store(true, Ordering::Release);
        let mut s = lock(&self.state);
        if s.status == Status::Open {
            s.status = status;
        }
        let resource = s.resource.take();
        // Remove bookkeeping before releasing capacity, so retained closed
        // aliases cannot cause registry growth beyond the socket limit.
        unregister(self);
        drop(resource);
        // Keep close serialized until the descriptor and permit are released.
        // A competing flight cleanup must not make another close return early.
        drop(s);
    }
    fn expire(&self, now: Instant) {
        let mut s = match self.state.try_lock() {
            Ok(s) => s,
            Err(std::sync::TryLockError::WouldBlock) => return,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        };
        if s.status == Status::Open && !s.active && now >= s.deadline {
            s.status = Status::Expired;
            let resource = s.resource.take();
            unregister(self);
            drop(resource);
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        unregister(self);
    }
}
fn unregister(owner: &Owner) {
    lock(registry()).retain(|w| !std::ptr::eq(w.as_ptr(), owner) && w.strong_count() > 0);
}
fn snapshot() -> Vec<Arc<Owner>> {
    lock(registry()).iter().filter_map(Weak::upgrade).collect()
}
fn sweep() {
    for owner in snapshot() {
        owner.expire(Instant::now());
    }
}
fn start_reaper() -> R<()> {
    REAPER
        .get_or_init(|| {
            std::thread::Builder::new()
                .name("ntnt-icmp-reaper".into())
                .spawn(|| loop {
                    std::thread::sleep(SWEEP);
                    sweep();
                })
                .map(|_| ())
                .map_err(|e| format!("backend: could not start ICMP reaper: {e}"))
        })
        .clone()
}
pub fn shutdown() {
    let owners = {
        let registry = lock(registry());
        // An open already in DNS/socket setup must not publish after shutdown.
        GENERATION.fetch_add(1, Ordering::AcqRel);
        registry
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    };
    for owner in owners {
        owner.terminate(Status::Closed);
    }
}
fn register(owner: &Arc<Owner>, generation: u64) -> R<()> {
    let mut owners = lock(registry());
    if GENERATION.load(Ordering::Acquire) != generation {
        return Err("cancelled: runtime shut down during ping_open".into());
    }
    owners.push(Arc::downgrade(owner));
    Ok(())
}
fn integer(v: Option<&Value>, default: i64, low: i64, high: i64) -> R<u64> {
    match v {
        None => Ok(default as u64),
        Some(Value::Int(n)) if (low..=high).contains(n) => Ok(*n as u64),
        _ => Err(format!(
            "invalid_argument: expected integer in {low}..{high}"
        )),
    }
}
pub fn open(args: &[Value]) -> R<Value> {
    let generation = GENERATION.load(Ordering::Acquire);
    if !(1..=2).contains(&args.len()) {
        return Err("invalid_argument: ping_open expects target and optional options".into());
    }
    let Value::String(host) = &args[0] else {
        return Err("invalid_argument: target must be String".into());
    };
    if host.is_empty() {
        return Err("invalid_argument: target must not be empty".into());
    }
    let mut idle = 60_000;
    let mut allow_private = false;
    if let Some(options) = args.get(1) {
        let Value::Map(options) = options else {
            return Err("invalid_argument: options must be Map".into());
        };
        for (key, value) in options {
            match key.as_str() {
                "idle_timeout_ms" => idle = integer(Some(value), 60_000, 1000, 86_400_000)?,
                "allow_private" => match value {
                    Value::Bool(v) => allow_private = *v,
                    _ => return Err("invalid_argument: allow_private must be Bool".into()),
                },
                _ => {
                    return Err(format!(
                        "invalid_argument: unsupported ping_open option {key}"
                    ))
                }
            }
        }
    }
    let targets = icmp::resolve_probe_targets(host).map_err(|e| format!("backend: {e}"))?;
    super::enforce_resolved_target_policy(&targets, allow_private)
        .map_err(|e| format!("policy: {e}"))?;
    let target = icmp::unique_target_ips(&targets).map_err(|e| format!("backend: {e}"))?[0];
    if crate::stdlib::concurrent::is_current_task_cancelled() {
        return Err("cancelled: owning task cancelled".into());
    }
    let permit = Permit::reserve()?;
    start_reaper()?;
    // Deliberately no fallback to another address after selecting the first.
    let socket =
        EchoSocket::open(target, Duration::from_secs(1)).map_err(|e| format!("backend: {e}"))?;
    socket
        .socket
        .set_nonblocking(true)
        .map_err(|e| format!("backend: {e}"))?;
    let owner = Arc::new(Owner {
        closing: AtomicBool::new(false),
        context: CONTEXT.with(Cell::get),
        nonce: rand::random(),
        idle: Duration::from_millis(idle),
        state: Mutex::new(State {
            resource: Some(Resource {
                socket,
                _permit: permit,
            }),
            status: Status::Open,
            active: false,
            deadline: Instant::now() + Duration::from_millis(idle),
            logical: 0,
        }),
    });
    if crate::stdlib::concurrent::is_current_task_cancelled() {
        return Err("cancelled: owning task cancelled during ping_open".into());
    }
    register(&owner, generation)?;
    Ok(Value::ProbeHandle(owner))
}
fn owner(args: &[Value]) -> R<&Arc<Owner>> {
    let Some(Value::ProbeHandle(owner)) = args.first() else {
        return Err("invalid_argument: expected native ProbeHandle".into());
    };
    if owner.context != CONTEXT.with(Cell::get) {
        return Err("ownership: ProbeHandle belongs to another owner".into());
    }
    Ok(owner)
}
pub fn close(args: &[Value]) -> R<Value> {
    if args.len() != 1 {
        return Err("invalid_argument: ping_close expects one handle".into());
    }
    owner(args)?.terminate(Status::Closed);
    Ok(Value::Unit)
}
/// Ensures panic, cancellation, and backend error cannot leave an active lease.
struct Flight<'a>(&'a Owner, bool);
impl Drop for Flight<'_> {
    fn drop(&mut self) {
        if !self.1 {
            self.0.terminate(Status::Closed);
        }
    }
}
pub fn probe(args: &[Value]) -> R<Value> {
    if !(1..=2).contains(&args.len()) {
        return Err("invalid_argument: ping_probe expects handle and optional timeout".into());
    }
    let timeout = Duration::from_millis(integer(args.get(1), 1000, 50, 30_000)?);
    let owner = owner(args)?;
    let started = Instant::now();
    let end = started + timeout;
    let (logical, sequence, target, payload) = {
        let mut s = match owner.state.try_lock() {
            Ok(s) => s,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err("busy: probe operation in progress".into());
            }
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        };
        match s.status {
            Status::Closed => return Err("closed: ProbeHandle is closed".into()),
            Status::Expired => return Err("expired: ProbeHandle idle lease ended".into()),
            Status::Open => {}
        }
        if s.active {
            return Err("busy: probe already in flight".into());
        }
        if owner.closing.load(Ordering::Acquire) {
            return Err("closed: ProbeHandle is closing".into());
        }
        if Instant::now() >= s.deadline {
            s.status = Status::Expired;
            let resource = s.resource.take();
            unregister(owner);
            drop(resource);
            return Err("expired: ProbeHandle idle lease ended".into());
        }
        if crate::stdlib::concurrent::is_current_task_cancelled() {
            drop(s);
            owner.terminate(Status::Closed);
            return Err("cancelled: owning task cancelled".into());
        }
        let Some(next) = s.logical.checked_add(1).filter(|n| *n <= i64::MAX as u64) else {
            drop(s);
            owner.terminate(Status::Closed);
            return Err("backend: logical probe counter exhausted".into());
        };
        s.logical = next;
        let logical = s.logical;
        let sequence = logical as u16;
        let mut payload = Vec::from(owner.nonce);
        payload.extend_from_slice(&logical.to_be_bytes());
        let target = s.resource.as_ref().expect("open resource").socket.target;
        s.active = true;
        (logical, sequence, target, payload)
    };
    let mut flight = Flight(owner, false);
    let sent_at = Instant::now();
    {
        let s = lock(&owner.state);
        let Some(resource) = &s.resource else {
            return Err("cancelled: probe closed before send".into());
        };
        if Instant::now() >= end {
            return Err("backend: probe deadline elapsed before send".into());
        }
        if owner.closing.load(Ordering::Acquire) {
            return Err("cancelled: probe closed before send".into());
        }
        resource
            .socket
            .send_persistent(sequence, &payload)
            .map_err(|e| format!("backend: {e}"))?;
    }
    let event = loop {
        if owner.closing.load(Ordering::Acquire) {
            return Err("cancelled: probe closed".into());
        }
        if crate::stdlib::concurrent::is_current_task_cancelled() {
            return Err("cancelled: owning task cancelled".into());
        }
        {
            let s = lock(&owner.state);
            let Some(resource) = &s.resource else {
                return Err("cancelled: probe closed".into());
            };
            if owner.closing.load(Ordering::Acquire) {
                return Err("cancelled: probe closed".into());
            }
            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break None;
            }
            match resource.socket.receive_persistent(
                sequence,
                &payload,
                sent_at,
                POLL.min(remaining),
            ) {
                Ok(Some(event)) if Instant::now() < end => break Some(event),
                Ok(Some(_)) => break None,
                Ok(None) => {}
                Err(e) => return Err(format!("backend: {e}")),
            }
        }
        // recv wakes as soon as data arrives. Drain unrelated/queued packets
        // immediately; never insert a polling delay into measured RTT.
    };
    {
        let mut s = lock(&owner.state);
        if s.status != Status::Open || owner.closing.load(Ordering::Acquire) {
            return Err("cancelled: probe closed".into());
        }
        s.active = false;
        s.deadline = Instant::now() + owner.idle;
        flight.1 = true;
    }
    let mut map = HashMap::from([
        ("probe_id".into(), Value::Int(logical as i64)),
        ("seq".into(), Value::Int(sequence as i64)),
        ("target_addr".into(), Value::String(target.to_string())),
        ("reachable".into(), Value::Bool(false)),
        ("status".into(), Value::String("timeout".into())),
    ]);
    match event {
        Some(IcmpProbeEvent::Reply(r)) => {
            map.insert("status".into(), Value::String("reply".into()));
            map.insert("reachable".into(), Value::Bool(true));
            map.insert("from".into(), Value::String(r.source.to_string()));
            map.insert("latency_ms".into(), Value::Float(r.latency_ms));
            if let Some(ttl) = r.ttl {
                map.insert("ttl".into(), Value::Int(ttl));
            }
        }
        Some(IcmpProbeEvent::Error(e)) => {
            map.insert("status".into(), Value::String("target_error".into()));
            map.insert("from".into(), Value::String(e.source.to_string()));
            map.insert("error".into(), Value::String(e.message()));
        }
        None => {}
    }
    Ok(Value::Map(map))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::interpreter::{ExecutionMode, Interpreter};
    use std::net::UdpSocket;
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

    // This is a UDP transport seam carrying ICMP bytes, NOT real ICMP evidence.
    fn fake(idle: Duration) -> (Arc<Owner>, UdpSocket) {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.connect(peer.local_addr().unwrap()).unwrap();
        socket.set_nonblocking(true).unwrap();
        let owner = Arc::new(Owner {
            closing: AtomicBool::new(false),
            context: CONTEXT.with(Cell::get),
            nonce: rand::random(),
            idle,
            state: Mutex::new(State {
                resource: Some(Resource {
                    socket: EchoSocket::test_udp(socket),
                    _permit: Permit::reserve().unwrap(),
                }),
                status: Status::Open,
                active: false,
                deadline: Instant::now() + idle,
                logical: 0,
            }),
        });
        start_reaper().unwrap();
        lock(registry()).push(Arc::downgrade(&owner));
        (owner, peer)
    }
    fn value(owner: &Arc<Owner>) -> Value {
        Value::ProbeHandle(owner.clone())
    }
    fn map(v: Value) -> HashMap<String, Value> {
        let Value::Map(m) = v else { panic!("not map") };
        m
    }
    fn until(mut condition: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(Instant::now() < end, "condition timed out");
            std::thread::sleep(POLL);
        }
    }
    #[test]
    fn queued_unrelated_packets_do_not_add_a_poll_delay_per_packet() {
        let _serial = lock(&SERIAL);
        let (owner, peer) = fake(Duration::from_secs(60));
        let responder = std::thread::spawn(move || {
            let mut bytes = [0; 128];
            let (len, addr) = peer.recv_from(&mut bytes).unwrap();
            // A raw socket sees its own outgoing echo, and can see other traffic.
            // All of these packets precede an already-queued matching reply.
            for _ in 0..16 {
                peer.send_to(&bytes[..len], addr).unwrap();
            }
            bytes[0] = 0;
            peer.send_to(&bytes[..len], addr).unwrap();
        });
        let result = map(probe(&[value(&owner), Value::Int(50)]).unwrap());
        responder.join().unwrap();
        close(&[value(&owner)]).unwrap();
        assert_eq!(result["status"].to_string(), "reply");
    }

    #[test]
    fn retained_alias_reaped_without_api_and_entry_cannot_revive() {
        let _serial = lock(&SERIAL);
        let before = LIVE.load(Ordering::Acquire);
        let (owner, _peer) = fake(Duration::from_millis(30));
        let alias = value(&owner);
        until(|| LIVE.load(Ordering::Acquire) == before);
        assert_eq!(lock(&owner.state).status, Status::Expired);
        assert!(probe(std::slice::from_ref(&alias))
            .unwrap_err()
            .starts_with("expired:"));
        close(std::slice::from_ref(&alias)).unwrap();
        close(&[alias]).unwrap();
        assert!(!lock(registry())
            .iter()
            .any(|w| std::ptr::eq(w.as_ptr(), owner.as_ref())));
        let (owner, _peer) = fake(Duration::from_secs(60));
        lock(&owner.state).deadline = Instant::now() - Duration::from_secs(1);
        assert!(probe(&[value(&owner)]).unwrap_err().starts_with("expired:"));
        assert!(lock(&owner.state).resource.is_none());
    }
    #[test]
    fn busy_entry_does_not_queue_or_renew_and_later_entry_checks_expiry() {
        let _serial = lock(&SERIAL);
        let (owner, _peer) = fake(Duration::from_secs(60));
        let mut state = lock(&owner.state);
        state.deadline = Instant::now() + Duration::from_millis(30);
        let rendezvous = Arc::new(std::sync::Barrier::new(2));
        let ready = rendezvous.clone();
        let active = owner.clone();
        let thread = std::thread::spawn(move || {
            ready.wait();
            probe(&[value(&active), Value::Int(50)]).map(|_| ())
        });
        rendezvous.wait();
        assert!(thread.join().unwrap().unwrap_err().starts_with("busy:"));
        std::thread::sleep(Duration::from_millis(80));
        drop(state);
        assert!(probe(&[value(&owner)]).unwrap_err().starts_with("expired:"));
        assert!(lock(&owner.state).resource.is_none());
    }
    #[test]
    fn active_protected_busy_close_cancels_and_releases_exactly_once() {
        let _serial = lock(&SERIAL);
        let before = LIVE.load(Ordering::Acquire);
        let (owner, peer) = fake(Duration::from_secs(1));
        let active = owner.clone();
        let thread =
            std::thread::spawn(move || probe(&[value(&active), Value::Int(30_000)]).map(|_| ()));
        let mut bytes = [0; 128];
        peer.recv_from(&mut bytes).unwrap();
        lock(&owner.state).deadline = Instant::now() - Duration::from_secs(1);
        sweep();
        assert!(lock(&owner.state).resource.is_some());
        assert!(probe(&[value(&owner)]).unwrap_err().starts_with("busy:"));
        let start = Instant::now();
        close(&[value(&owner)]).unwrap();
        assert!(start.elapsed() < Duration::from_millis(50));
        assert_eq!(LIVE.load(Ordering::Acquire), before);
        assert!(thread
            .join()
            .unwrap()
            .unwrap_err()
            .starts_with("cancelled:"));
        close(&[value(&owner)]).unwrap();
        assert_eq!(LIVE.load(Ordering::Acquire), before);
    }
    #[test]
    fn same_socket_sequence_rollover_duplicate_rejection_and_timeout_renewal() {
        let _serial = lock(&SERIAL);
        let (owner, peer) = fake(Duration::from_secs(60));
        lock(&owner.state).logical = 65_534;
        let responder = std::thread::spawn(move || {
            let mut buf = [0; 128];
            let (n, addr) = peer.recv_from(&mut buf).unwrap();
            let first = buf[..n].to_vec();
            assert_eq!(&first[6..8], &u16::MAX.to_be_bytes());
            buf[0] = 0;
            peer.send_to(&buf[..n], addr).unwrap();
            let (n, next_addr) = peer.recv_from(&mut buf).unwrap();
            assert_eq!(next_addr, addr, "same UDP socket seam address");
            assert_eq!(&buf[6..8], &[0, 0]);
            assert_ne!(&buf[8..n], &first[8..]);
            // Forge the old wire sequence to current: generation must still reject.
            let mut stale = first;
            stale[0] = 0;
            stale[6..8].copy_from_slice(&[0, 0]);
            peer.send_to(&stale, addr).unwrap();
            // No current reply: verifies timeout isn't masked by old payload.
            std::thread::sleep(Duration::from_millis(150));
        });
        let r = map(probe(&[value(&owner), Value::Int(100)]).unwrap());
        assert_eq!(r["status"].to_string(), "reply");
        assert_eq!(r["probe_id"].to_string(), "65535");
        let previous_deadline = lock(&owner.state).deadline;
        let r = map(probe(&[value(&owner), Value::Int(100)]).unwrap());
        assert_eq!(r["status"].to_string(), "timeout");
        assert_eq!(r["seq"].to_string(), "0");
        assert_eq!(r["probe_id"].to_string(), "65536");
        assert!(!r.contains_key("latency_ms"));
        assert!(lock(&owner.state).deadline > previous_deadline);
        responder.join().unwrap();
        close(&[value(&owner)]).unwrap();
    }
    #[test]
    fn correlated_target_error_is_measurement_and_renews_lease() {
        let _serial = lock(&SERIAL);
        let (owner, peer) = fake(Duration::from_secs(60));
        let previous = lock(&owner.state).deadline;
        let responder = std::thread::spawn(move || {
            let mut buffer = [0; 128];
            let (len, addr) = peer.recv_from(&mut buffer).unwrap();
            let mut quote = vec![0; 20];
            quote[0] = 0x45;
            quote[9] = 1;
            quote[16..20].copy_from_slice(&[127, 0, 0, 1]);
            quote.extend_from_slice(&buffer[..len]);
            let mut error = vec![3, 1, 0, 0, 0, 0, 0, 0];
            error.extend_from_slice(&quote);
            // Header-only quote is ambiguous and must not complete this probe.
            peer.send_to(&error[..36], addr).unwrap();
            peer.send_to(&error, addr).unwrap();
        });
        let result = map(probe(&[value(&owner), Value::Int(500)]).unwrap());
        responder.join().unwrap();
        assert_eq!(result["status"].to_string(), "target_error");
        assert_eq!(result["reachable"].to_string(), "false");
        assert_eq!(result["from"].to_string(), "127.0.0.1");
        assert!(result["error"].to_string().contains("type 3 code 1"));
        assert!(!result.contains_key("latency_ms"));
        assert!(lock(&owner.state).deadline > previous);
        assert_eq!(lock(&owner.state).status, Status::Open);
        close(&[value(&owner)]).unwrap();
    }
    #[test]
    fn independent_handles_local_send_failure_and_unwind_cleanup() {
        let _serial = lock(&SERIAL);
        let before = LIVE.load(Ordering::Acquire);
        let (a, peer) = fake(Duration::from_secs(60));
        let (b, _peer) = fake(Duration::from_secs(60));
        let active = a.clone();
        let thread =
            std::thread::spawn(move || probe(&[value(&active), Value::Int(500)]).map(|_| ()));
        let mut bytes = [0; 128];
        peer.recv_from(&mut bytes).unwrap();
        assert_eq!(
            map(probe(&[value(&b), Value::Int(50)]).unwrap())["status"].to_string(),
            "timeout"
        );
        close(&[value(&a)]).unwrap();
        thread.join().unwrap().unwrap_err();
        // Unconnected UDP socket makes send() fail locally; never a timeout result.
        lock(&b.state).resource.as_mut().unwrap().socket =
            EchoSocket::test_udp(UdpSocket::bind("127.0.0.1:0").unwrap());
        assert!(probe(&[value(&b)]).unwrap_err().starts_with("backend:"));
        assert!(lock(&b.state).resource.is_none());
        let (c, _peer) = fake(Duration::from_secs(60));
        let _ = std::panic::catch_unwind(|| {
            let _flight = Flight(&c, false);
            panic!("flight unwind");
        });
        assert!(lock(&c.state).resource.is_none());
        assert_eq!(LIVE.load(Ordering::Acquire), before);
    }
    #[test]
    fn capacity_partial_failure_drop_and_owner_isolation() {
        let _serial = lock(&SERIAL);
        let before = LIVE.load(Ordering::Acquire);
        let permits = (before..LIMIT)
            .map(|_| Permit::reserve().unwrap())
            .collect::<Vec<_>>();
        assert!(Permit::reserve().err().unwrap().starts_with("capacity:"));
        drop(permits);
        assert_eq!(LIVE.load(Ordering::Acquire), before);
        let (a, _peer) = fake(Duration::from_secs(60));
        let scope = Scope::new();
        let context = scope.enter();
        let (b, _peer2) = fake(Duration::from_secs(60));
        drop(context);
        assert!(probe(&[value(&b)]).unwrap_err().starts_with("ownership:"));
        drop(scope);
        assert!(lock(&b.state).resource.is_none());
        assert!(lock(&a.state).resource.is_some());
        drop(a);
        drop(b);
        assert_eq!(LIVE.load(Ordering::Acquire), before);
    }
    pub(crate) fn install_fake(interp: &mut Interpreter) {
        interp.define_in_scope(
            "test_probe_fail".into(),
            Value::NativeFunction {
                name: "test_probe_fail".into(),
                arity: 0,
                max_arity: 0,
                requires: None,
                func: |_| {
                    Err(crate::error::IntentError::runtime_error(
                        "probe task error fixture",
                    ))
                },
            },
        );
        interp.define_in_scope(
            "test_probe_open".into(),
            Value::NativeFunction {
                name: "test_probe_open".into(),
                arity: 0,
                max_arity: 0,
                requires: None,
                func: |_| {
                    let (owner, _peer) = fake(Duration::from_secs(60));
                    Ok(value(&owner))
                },
            },
        );
    }
    pub(crate) fn live() -> usize {
        LIVE.load(Ordering::Acquire)
    }
    pub(crate) fn eval(interp: &mut Interpreter, source: &str) -> crate::error::Result<Value> {
        let ast = crate::parser::Parser::new(crate::lexer::Lexer::new(source).collect())
            .parse()
            .unwrap();
        interp.eval(&ast)
    }
    #[test]
    fn interpreter_and_task_scope_dispose_retained_cycles_and_errors() {
        let _serial = lock(&SERIAL);
        let before = live();
        for mode in [
            ExecutionMode::Normal,
            ExecutionMode::Worker,
            ExecutionMode::Job,
        ] {
            let mut interp = Interpreter::new();
            interp.set_execution_mode(mode);
            install_fake(&mut interp);
            let retained = eval(
                &mut interp,
                "let h = test_probe_open()\nlet cycle = fn() { h }\nh",
            )
            .unwrap();
            assert_eq!(live(), before + 1);
            drop(interp);
            let Value::ProbeHandle(h) = retained else {
                panic!()
            };
            assert!(lock(&h.state).resource.is_none());
            assert_eq!(live(), before);
        }
        let mut interp = Interpreter::new();
        install_fake(&mut interp);
        eval(
            &mut interp,
            r#"
import { spawn, await_task } from "std/concurrent"
let task = spawn(fn() { let h = test_probe_open()
let cycle = fn() { h }
test_probe_fail() })
await_task(task)
"#,
        )
        .unwrap();
        assert_eq!(
            live(),
            before,
            "task error must drop interpreter scope despite cycle"
        );
    }
    #[test]
    fn task_cancellation_and_return_transfer_release_scope() {
        let _serial = lock(&SERIAL);
        let before = live();
        let mut interp = Interpreter::new();
        install_fake(&mut interp);
        eval(
            &mut interp,
            r#"
import { spawn, await_task, cancel_task, sleep_ms } from "std/concurrent"
let task = spawn(fn() { let h = test_probe_open()
let cycle = fn() { h }
sleep_ms(30000)
true })
"#,
        )
        .unwrap();
        until(|| live() == before + 1);
        let result = eval(&mut interp, "cancel_task(task)\nawait_task(task)").unwrap();
        assert!(result.to_string().contains("cancel"), "{result}");
        assert_eq!(live(), before);
        let result = eval(
            &mut interp,
            "let returned = spawn(fn() { test_probe_open() })\nawait_task(returned)",
        )
        .unwrap();
        assert!(result.to_string().contains("non-serializable"), "{result}");
        assert_eq!(live(), before);
        // Captured authority is rejected before a task can start.
        let error = eval(&mut interp, "let h = test_probe_open()\nspawn(fn() { h })").unwrap_err();
        assert!(error.to_string().contains("runtime-local"));
        drop(interp);
        assert_eq!(live(), before);
    }
    #[test]
    fn real_socket_identity_reclaim_or_permission_rollback() {
        let _serial = lock(&SERIAL);
        let before = live();
        let prior = std::env::var_os("NTNT_NET_ALLOW_PRIVATE");
        std::env::set_var("NTNT_NET_ALLOW_PRIVATE", "1");
        let opened = open(&[
            Value::String("127.0.0.1".into()),
            Value::Map(HashMap::from([
                ("allow_private".into(), Value::Bool(true)),
                ("idle_timeout_ms".into(), Value::Int(1000)),
            ])),
        ]);
        if let Some(prior) = prior {
            std::env::set_var("NTNT_NET_ALLOW_PRIVATE", prior);
        } else {
            std::env::remove_var("NTNT_NET_ALLOW_PRIVATE");
        }
        let h = match opened {
            Ok(h) => h,
            Err(error) => {
                assert_eq!(live(), before, "failed setup leaked reservation");
                assert!(error.starts_with("backend:"), "{error}");
                assert_ne!(
                    std::env::var("NTNT_ICMP_REQUIRE").as_deref(),
                    Ok("1"),
                    "{error}"
                );
                eprintln!("SKIP real socket identity/reclaim: {error}; capacity rollback asserted, no ICMP probes asserted");
                return;
            }
        };
        let Value::ProbeHandle(owner) = &h else {
            panic!()
        };
        let address = lock(&owner.state)
            .resource
            .as_ref()
            .unwrap()
            .socket
            .socket
            .local_addr()
            .unwrap();
        #[cfg(unix)]
        let fd = {
            use std::os::fd::AsRawFd;
            lock(&owner.state)
                .resource
                .as_ref()
                .unwrap()
                .socket
                .socket
                .as_raw_fd()
        };
        for seq in 1..=3 {
            let result = map(probe(std::slice::from_ref(&h)).unwrap());
            assert_eq!(result["status"].to_string(), "reply");
            assert_eq!(result["seq"].to_string(), seq.to_string());
            assert_eq!(
                lock(&owner.state)
                    .resource
                    .as_ref()
                    .unwrap()
                    .socket
                    .socket
                    .local_addr()
                    .unwrap(),
                address
            );
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                assert_eq!(
                    lock(&owner.state)
                        .resource
                        .as_ref()
                        .unwrap()
                        .socket
                        .socket
                        .as_raw_fd(),
                    fd
                );
            }
        }
        until(|| live() == before);
        assert!(lock(&owner.state).resource.is_none());
        #[cfg(target_os = "linux")]
        assert!(!std::path::Path::new(&format!("/proc/self/fd/{fd}")).exists());
        assert!(probe(&[h]).unwrap_err().starts_with("expired:"));
        eprintln!("REAL ICMP: same descriptor/address across three probes; retained handle reaped without API activity");
    }
    #[test]
    fn opaque_nested_boundaries_and_invalid_calls_do_not_renew() {
        let _serial = lock(&SERIAL);
        let (owner, _peer) = fake(Duration::from_secs(60));
        let nested = Value::Map(HashMap::from([(
            "nested".into(),
            Value::Array(vec![value(&owner)]),
        )]));
        assert!(crate::stdlib::json::intent_value_to_json_reject(&nested).is_err());
        assert!(crate::stdlib::concurrent::SerializedValue::from_value(&nested).is_err());
        let deadline = lock(&owner.state).deadline;
        assert!(probe(&[value(&owner), Value::Int(0)]).is_err());
        assert_eq!(lock(&owner.state).deadline, deadline);
        close(&[value(&owner)]).unwrap();
    }
    #[test]
    fn denied_modes_and_native_observer_fail_closed_through_aliases_and_callbacks() {
        for observer in [false, true] {
            for mode in [
                ExecutionMode::Normal,
                ExecutionMode::Worker,
                ExecutionMode::Job,
                ExecutionMode::HotReload,
                ExecutionMode::UnitTest,
            ] {
                if !observer
                    && matches!(
                        mode,
                        ExecutionMode::Normal | ExecutionMode::Worker | ExecutionMode::Job
                    )
                {
                    continue;
                }
                for call in [
                    r#"alias("127.0.0.1")"#,
                    r#"sort_by(["127.0.0.1", "127.0.0.1"], alias)"#,
                ] {
                    let mut interp = Interpreter::new();
                    if observer {
                        interp.configure_native_test("test_entry");
                    }
                    interp.set_execution_mode(mode);
                    let source = format!("import {{ ping_open }} from \"std/net\"\nimport {{ sort_by }} from \"std/collections\"\nlet alias = ping_open\n{call}");
                    assert!(eval(&mut interp, &source)
                        .unwrap_err()
                        .to_string()
                        .contains("capability:"));
                }
            }
        }
    }
    #[test]
    fn runtime_shutdown_rejects_partial_open_publication() {
        let _serial = lock(&SERIAL);
        let before = live();
        let (owner, _peer) = fake(Duration::from_secs(60));
        unregister(&owner); // Simulate the socket-setup portion of open, before publication.
        let generation = GENERATION.load(Ordering::Acquire);
        shutdown();
        assert!(register(&owner, generation)
            .unwrap_err()
            .starts_with("cancelled:"));
        drop(owner);
        assert_eq!(live(), before);
    }
    #[test]
    fn runtime_shutdown_disposes_active_and_idle_handles() {
        let _serial = lock(&SERIAL);
        let (a, peer) = fake(Duration::from_secs(60));
        let (b, _peer) = fake(Duration::from_secs(60));
        let active = a.clone();
        let thread =
            std::thread::spawn(move || probe(&[value(&active), Value::Int(30_000)]).map(|_| ()));
        let mut bytes = [0; 128];
        peer.recv_from(&mut bytes).unwrap();
        shutdown();
        assert!(thread
            .join()
            .unwrap()
            .unwrap_err()
            .starts_with("cancelled:"));
        assert!(lock(&a.state).resource.is_none());
        assert!(lock(&b.state).resource.is_none());
        assert!(lock(registry()).is_empty());
    }
}
