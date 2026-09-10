//! Bounded server-side sockets. No outbound connect authority is exposed.
use crate::interpreter::Value;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

type R<T> = Result<T, String>;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static BUFFERS: AtomicUsize = AtomicUsize::new(0);
static OWNERS: OnceLock<Mutex<Vec<Weak<Owner>>>> = OnceLock::new();
const HANDLE_LIMIT: usize = 128;
const BUFFER_LIMIT: usize = 16 * 1024 * 1024;
struct Permit {
    counter: &'static AtomicUsize,
    amount: usize,
}
impl Permit {
    fn reserve(counter: &'static AtomicUsize, amount: usize, limit: usize) -> R<Self> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(amount).filter(|n| *n <= limit)
            })
            .map_err(|_| "capacity: TCP resource limit".to_string())?;
        Ok(Self { counter, amount })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.amount, Ordering::AcqRel);
    }
}
enum Socket {
    Listener(TcpListener),
    Stream(TcpStream),
}
struct Open {
    socket: Socket,
    write_shutdown: bool,
    _permit: Permit,
}
pub struct Owner {
    state: Mutex<Option<Open>>,
}
impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<opaque TCP socket>")
    }
}
impl Owner {
    fn lock(&self) -> R<MutexGuard<'_, Option<Open>>> {
        self.state.try_lock().map_err(|e| match e {
            std::sync::TryLockError::WouldBlock => "busy: socket operation in progress".into(),
            std::sync::TryLockError::Poisoned(_) => "system: poisoned socket owner".into(),
        })
    }
    fn new(socket: Socket, permit: Permit) -> R<Arc<Self>> {
        let owner = Arc::new(Self {
            state: Mutex::new(Some(Open {
                socket,
                write_shutdown: false,
                _permit: permit,
            })),
        });
        let mut owners = OWNERS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map_err(|_| "system: poisoned socket registry".to_string())?;
        owners.retain(|w| w.strong_count() != 0);
        owners.push(Arc::downgrade(&owner));
        Ok(owner)
    }
}
fn unregister(owner: &Owner) -> R<()> {
    if let Some(owners) = OWNERS.get() {
        owners
            .lock()
            .map_err(|_| "system: poisoned TCP registry during cleanup".to_string())?
            .retain(|w| !std::ptr::eq(w.as_ptr(), owner));
    }
    Ok(())
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Err(error) = unregister(self) {
            // Drop cannot return an error. Report best-effort without panicking;
            // descriptor/permit fields still drop even when bookkeeping is poisoned.
            let _ = writeln!(io::stderr(), "TCP cleanup failed: {error}");
        }
    }
}
pub fn shutdown() -> R<()> {
    let snapshot = OWNERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "system: poisoned TCP shutdown registry".to_string())?
        .iter()
        .filter_map(Weak::upgrade)
        .collect::<Vec<_>>();
    let end = Instant::now() + Duration::from_secs(60);
    for owner in snapshot {
        loop {
            match owner.lock() {
                Ok(mut state) => {
                    let open = state.take();
                    drop(state);
                    drop(open);
                    unregister(&owner)?;
                    break;
                }
                Err(e) if e.starts_with("busy:") => retry(end)?,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

fn integer(value: Option<&Value>, default: i64, low: i64, high: i64) -> R<i64> {
    match value {
        None => Ok(default),
        Some(Value::Int(n)) if (low..=high).contains(n) => Ok(*n),
        _ => Err(format!(
            "invalid_argument: integer must be in {low}..{high}"
        )),
    }
}
fn deadline(value: Option<&Value>) -> R<Instant> {
    Ok(Instant::now() + Duration::from_millis(integer(value, 5000, 1, 60000)? as u64))
}
fn retry(end: Instant) -> R<()> {
    let remaining = end
        .checked_duration_since(Instant::now())
        .ok_or("timeout: TCP operation deadline")?;
    std::thread::sleep(remaining.min(Duration::from_millis(1)));
    Ok(())
}
fn io_error(e: io::Error) -> String {
    format!("io: {e}")
}
fn owner(value: &Value) -> R<&Arc<Owner>> {
    match value {
        Value::TcpListener(o) | Value::TcpStream(o) => Ok(o),
        _ => Err("invalid_argument: expected native TCP socket".into()),
    }
}
fn stream_owner(value: &Value) -> R<&Arc<Owner>> {
    match value {
        Value::TcpStream(o) => Ok(o),
        _ => Err("invalid_argument: expected native TcpStream".into()),
    }
}
fn address(addr: SocketAddr) -> Value {
    Value::Map(HashMap::from([
        ("host".into(), Value::String(addr.ip().to_string())),
        ("port".into(), Value::Int(i64::from(addr.port()))),
    ]))
}
pub fn listen(args: &[Value]) -> R<Value> {
    let port = integer(args.first(), 0, 0, 65535)? as u16;
    let mut host: IpAddr = std::net::Ipv4Addr::LOCALHOST.into();
    if let Some(options) = args.get(1) {
        let Value::Map(options) = options else {
            return Err("invalid_argument: options must be Map".into());
        };
        for (key, value) in options {
            if key != "host" {
                return Err("invalid_argument: unknown tcp_listen option".into());
            }
            let Value::String(s) = value else {
                return Err("invalid_argument: host must be literal IP".into());
            };
            host = s
                .parse()
                .map_err(|_| "invalid_argument: host must be literal IP".to_string())?;
        }
    }
    let permit = Permit::reserve(&LIVE, 1, HANDLE_LIMIT)?;
    let listener = TcpListener::bind(SocketAddr::new(host, port)).map_err(io_error)?;
    listener.set_nonblocking(true).map_err(io_error)?;
    Ok(Value::TcpListener(Owner::new(
        Socket::Listener(listener),
        permit,
    )?))
}
pub fn accept(args: &[Value]) -> R<Value> {
    let end = deadline(args.get(1))?;
    let Value::TcpListener(owner) = &args[0] else {
        return Err("invalid_argument: expected native TcpListener".into());
    };
    let state = owner.lock()?;
    let Some(Open {
        socket: Socket::Listener(listener),
        ..
    }) = state.as_ref()
    else {
        return Err("closed: listener".into());
    };
    let permit = Permit::reserve(&LIVE, 1, HANDLE_LIMIT)?;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(true).map_err(io_error)?;
                return Ok(Value::TcpStream(Owner::new(
                    Socket::Stream(stream),
                    permit,
                )?));
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                retry(end)?
            }
            Err(e) => return Err(io_error(e)),
        }
    }
}
pub fn read(args: &[Value]) -> R<Value> {
    let count = integer(args.get(1), 0, 1, 65536)? as usize;
    let end = deadline(args.get(2))?;
    let owner = stream_owner(&args[0])?;
    let mut state = owner.lock()?;
    let Some(Open {
        socket: Socket::Stream(stream),
        ..
    }) = state.as_mut()
    else {
        return Err("closed: stream".into());
    };
    let amount = count
        .checked_mul(1 + std::mem::size_of::<Value>())
        .ok_or("capacity: buffer overflow")?;
    let _buffer = Permit::reserve(&BUFFERS, amount, BUFFER_LIMIT)?;
    let mut bytes = vec![0; count];
    loop {
        match stream.read(&mut bytes) {
            Ok(0) => return Ok(Value::none()),
            Ok(n) => {
                return Ok(Value::some(Value::Array(
                    bytes[..n]
                        .iter()
                        .map(|b| Value::Int(i64::from(*b)))
                        .collect(),
                )))
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                retry(end)?
            }
            Err(e) => return Err(io_error(e)),
        }
    }
}
pub fn write(args: &[Value]) -> R<Value> {
    let end = deadline(args.get(2))?;
    let count = match &args[1] {
        Value::String(s) => s.len(),
        Value::Array(a) => {
            if a.len() > 65536
                || a.iter()
                    .any(|v| !matches!(v, Value::Int(n) if (0..=255).contains(n)))
            {
                return Err("invalid_argument: expected bytes in 0..255, at most 65536".into());
            }
            a.len()
        }
        _ => return Err("invalid_argument: expected String or Array<Int>".into()),
    };
    if count > 65536 {
        return Err("invalid_argument: write exceeds 65536 bytes".into());
    }
    let owner = stream_owner(&args[0])?;
    let mut state = owner.lock()?;
    if state.as_ref().is_some_and(|open| open.write_shutdown) {
        return Err("closed: stream write side is shut down".into());
    }
    let Some(Open {
        socket: Socket::Stream(stream),
        ..
    }) = state.as_mut()
    else {
        return Err("closed: stream".into());
    };
    let _buffer = Permit::reserve(&BUFFERS, count, BUFFER_LIMIT)?;
    let converted;
    let bytes = match &args[1] {
        Value::String(s) => s.as_bytes(),
        Value::Array(a) => {
            converted = a
                .iter()
                .map(|v| {
                    if let Value::Int(n) = v {
                        *n as u8
                    } else {
                        unreachable!()
                    }
                })
                .collect::<Vec<_>>();
            &converted
        }
        _ => unreachable!(),
    };
    let mut written = 0;
    while written < bytes.len() {
        let error = match stream.write(&bytes[written..]) {
            Ok(0) => Some("io: zero-length write".into()),
            Ok(n) => {
                written += n;
                if written < bytes.len() && Instant::now() >= end {
                    Some("timeout: write deadline".into())
                } else {
                    None
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                retry(end).err()
            }
            Err(e) => Some(io_error(e)),
        };
        if let Some(error) = error {
            let open = state.take();
            drop(state);
            drop(open);
            let mut message =
                format!("write_failed: stream_closed=true; bytes_written={written}; {error}");
            if let Err(cleanup) = unregister(owner) {
                message.push_str(&format!("; {cleanup}"));
            }
            return Err(message);
        }
    }
    Ok(Value::Int(written as i64))
}
pub fn local_addr(args: &[Value]) -> R<Value> {
    let state = owner(&args[0])?.lock()?;
    let open = state.as_ref().ok_or("closed: socket")?;
    Ok(address(
        match &open.socket {
            Socket::Listener(l) => l.local_addr(),
            Socket::Stream(s) => s.local_addr(),
        }
        .map_err(io_error)?,
    ))
}
pub fn peer_addr(args: &[Value]) -> R<Value> {
    let state = stream_owner(&args[0])?.lock()?;
    let Some(Open {
        socket: Socket::Stream(s),
        ..
    }) = state.as_ref()
    else {
        return Err("closed: stream".into());
    };
    Ok(address(s.peer_addr().map_err(io_error)?))
}
pub fn shutdown_stream(args: &[Value]) -> R<Value> {
    let how = match &args[1] {
        Value::String(s) => match s.as_str() {
            "read" => Shutdown::Read,
            "write" => Shutdown::Write,
            "both" => Shutdown::Both,
            _ => return Err("invalid_argument: shutdown expects read/write/both".into()),
        },
        _ => return Err("invalid_argument: shutdown expects String".into()),
    };
    let mut state = stream_owner(&args[0])?.lock()?;
    let Some(Open {
        socket: Socket::Stream(s),
        ..
    }) = state.as_ref()
    else {
        return Err("closed: stream".into());
    };
    s.shutdown(how).map_err(io_error)?;
    if matches!(how, Shutdown::Write | Shutdown::Both) {
        if let Some(open) = state.as_mut() {
            open.write_shutdown = true;
        }
    }
    Ok(Value::Unit)
}
pub fn close(args: &[Value]) -> R<Value> {
    let mut state = owner(&args[0])?.lock()?;
    let open = state.take();
    drop(state);
    drop(open);
    unregister(owner(&args[0])?)?;
    Ok(Value::Unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    #[test]
    fn poisoned_registry_returns_system_error_without_panicking_on_cleanup() {
        let _serial = TEST_LOCK.lock().unwrap();
        let listener = listen(&[Value::Int(0)]).unwrap();
        let registry = OWNERS.get().unwrap();
        let _ = std::panic::catch_unwind(|| {
            let _guard = registry.lock().unwrap();
            panic!("injected registry poison");
        });
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| close(&[listener.clone()])));
        // Restore only this test's deliberately poisoned registry before assertions/drop.
        registry.clear_poison();
        assert!(
            result.is_ok(),
            "socket cleanup must not panic on registry poison"
        );
        assert!(result.unwrap().unwrap_err().starts_with("system:"));
        close(&[listener]).unwrap();
        assert_eq!(LIVE.load(Ordering::Acquire), 0);
    }
    #[test]
    fn independent_socket_progress_during_pending_read() {
        let _serial = TEST_LOCK.lock().unwrap();
        let listener = listen(&[Value::Int(0)]).unwrap();
        let state = owner(&listener).unwrap().lock().unwrap();
        let Some(Open {
            socket: Socket::Listener(l),
            ..
        }) = state.as_ref()
        else {
            panic!()
        };
        let client = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        drop(state);
        let stream = accept(&[listener.clone()]).unwrap();
        let arc = stream_owner(&stream).unwrap().clone();
        std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                read(&[Value::TcpStream(arc), Value::Int(1), Value::Int(100)]).unwrap_err()
            });
            let end = Instant::now() + Duration::from_millis(50);
            loop {
                if stream_owner(&stream).unwrap().lock().is_err() {
                    break;
                }
                assert!(Instant::now() < end, "read never acquired owner");
                std::thread::sleep(Duration::from_millis(1));
            }
            let other = listen(&[Value::Int(0)]).unwrap();
            close(&[other]).unwrap();
            assert!(close(&[stream.clone()]).unwrap_err().starts_with("busy:"));
            assert!(worker.join().unwrap().starts_with("timeout:"));
        });
        drop(client);
        close(&[stream]).unwrap();
        close(&[listener]).unwrap();
    }
    #[test]
    fn buffer_budget_and_owner_busy_are_bounded() {
        let _serial = TEST_LOCK.lock().unwrap();
        let buffer = Permit::reserve(&BUFFERS, BUFFER_LIMIT, BUFFER_LIMIT).unwrap();
        assert!(Permit::reserve(&BUFFERS, 1, BUFFER_LIMIT).is_err());
        drop(buffer);
        assert_eq!(BUFFERS.load(Ordering::Acquire), 0);
        let listener = listen(&[Value::Int(0)]).unwrap();
        let own = owner(&listener).unwrap();
        let guard = own.lock().unwrap();
        assert!(close(&[listener.clone()]).unwrap_err().starts_with("busy:"));
        let independent = listen(&[Value::Int(0)]).unwrap();
        close(&[independent]).unwrap();
        drop(guard);
        close(&[listener]).unwrap();
    }
    #[test]
    fn shutdown_closes_aliases_and_clears_weak_tracking() {
        let _serial = TEST_LOCK.lock().unwrap();
        let listener = listen(&[Value::Int(0)]).unwrap();
        shutdown().unwrap();
        assert!(local_addr(&[listener.clone()])
            .unwrap_err()
            .starts_with("closed:"));
        close(&[listener]).unwrap();
        assert_eq!(LIVE.load(Ordering::Acquire), 0);
        assert!(OWNERS.get().unwrap().lock().unwrap().is_empty());
    }
    #[test]
    fn transfers_reject_nested_native_socket_authority() {
        let _serial = TEST_LOCK.lock().unwrap();
        let listener = listen(&[Value::Int(0)]).unwrap();
        let nested = Value::ok(Value::Array(vec![listener.clone()]));
        assert!(crate::stdlib::concurrent::SerializedValue::from_value(&nested).is_err());
        assert!(crate::stdlib::json::intent_value_to_json_reject(&nested).is_err());
        close(&[listener]).unwrap();
    }
    #[test]
    fn observer_denies_even_when_surrounding_mode_is_normal() {
        let _serial = TEST_LOCK.lock().unwrap();
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.configure_native_test("test_entry");
        interpreter.set_execution_mode(crate::interpreter::ExecutionMode::Normal);
        let source = "import { tcp_listen } from \"std/net\"\nlet alias = tcp_listen\nalias(0)";
        let program = crate::parser::Parser::new(crate::lexer::Lexer::new(source).collect())
            .parse()
            .unwrap();
        assert!(interpreter
            .eval(&program)
            .unwrap_err()
            .to_string()
            .contains("Unsupported native test capability"));
    }
}
