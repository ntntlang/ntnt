//! Owner-locked Unix worker control endpoints. See docs/worker-control.md.
//! Protocol: one newline-delimited JSON request and response per connection.

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::{Arc, LazyLock, Mutex};

/// Explicit options take precedence over NTNT_CONTROL_SOCKET / NTNT_WORKER_GROUP.
#[derive(Clone, Debug, Default)]
pub struct ControlOptions {
    pub control_socket: Option<PathBuf>,
    pub worker_group: Option<String>,
}

thread_local! {
    static SOURCE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Scope source identity to a native worker call, without changing process environment.
pub fn with_source<T>(source: Option<&Path>, call: impl FnOnce() -> T) -> T {
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SOURCE.with(|s| {
                s.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(SOURCE.with(|s| s.replace(source.map(Path::to_path_buf))));
    call()
}

/// Canonical nearest ntnt.toml ancestor, or the supplied source directory.
pub fn project_identity(context: &Path) -> std::io::Result<PathBuf> {
    let canonical = context.canonicalize()?;
    let directory = if canonical.is_file() {
        canonical.parent().unwrap()
    } else {
        &canonical
    };
    Ok(directory
        .ancestors()
        .find(|p| p.join("ntnt.toml").is_file())
        .unwrap_or(directory)
        .to_path_buf())
}

/// Resolve an endpoint identically for server and client. Relative explicit paths
/// are relative to canonical project identity, not the shell's working directory.
/// On Unix, creates/validates the private default runtime directory on first use.
pub fn resolve(context: &Path, options: &ControlOptions) -> std::io::Result<PathBuf> {
    let explicit = options
        .control_socket
        .clone()
        .or_else(|| std::env::var_os("NTNT_CONTROL_SOCKET").map(PathBuf::from));
    let group = options.worker_group.clone().or_else(|| {
        std::env::var_os("NTNT_WORKER_GROUP").map(|s| s.to_string_lossy().into_owned())
    });
    #[cfg(not(unix))]
    {
        let _ = (context, explicit, group);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Unix worker control sockets and worker groups are unsupported on Windows",
        ))
    }
    #[cfg(unix)]
    {
        use sha2::{Digest, Sha256};
        use std::os::unix::ffi::OsStrExt;
        let group = group.as_deref().unwrap_or("default");
        if group.is_empty() {
            return Err(invalid("worker group must not be empty"));
        }
        let path = if let Some(path) = explicit {
            if path.as_os_str().is_empty() {
                return Err(invalid("control socket path must not be empty"));
            }
            let path = if path.is_absolute() {
                path
            } else {
                project_identity(context)?.join(path)
            };
            let parent = path
                .parent()
                .ok_or_else(|| invalid("control socket requires a parent directory"))?;
            let parent = parent
                .canonicalize()
                .map_err(|e| endpoint_error(&path, e))?;
            parent.join(
                path.file_name()
                    .ok_or_else(|| invalid("control socket requires a filename"))?,
            )
        } else {
            let project = project_identity(context)?;
            let mut hash = Sha256::new();
            hash.update(project.as_os_str().as_bytes());
            hash.update([0]);
            hash.update(group.as_bytes());
            runtime_directory()?.join(format!("{}.sock", hex::encode(&hash.finalize()[..20])))
        };
        // macOS has the smallest supported sockaddr_un pathname (104 bytes).
        if path.as_os_str().as_bytes().len() > 103 || path.as_os_str().as_bytes().contains(&0) {
            return Err(endpoint_error(
                &path,
                "Unix socket path must be at most 103 bytes and contain no NUL",
            ));
        }
        validate_parent(&path)?;
        Ok(path)
    }
}

#[cfg(unix)]
fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
#[cfg(unix)]
fn endpoint_error(path: &Path, error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(format!("control socket {}: {error}", path.display()))
}

#[cfg(unix)]
fn uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn validate_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != uid() || m.mode() & 0o077 != 0 {
        return Err(endpoint_error(path, "directory must be owned by the caller and have safe permissions (runtime and explicit parent: 0700)"));
    }
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(endpoint_error(path, e)),
    }
    validate_directory(path)
}

#[cfg(unix)]
fn runtime_directory() -> std::io::Result<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        if base.is_absolute()
            && base.as_os_str().len() + "/ntnt/".len() + 40 + ".sock".len() <= 103
            && validate_directory(&base).is_ok()
        {
            let path = base.join("ntnt");
            private_directory(&path)?;
            return Ok(path);
        }
    }
    // A short, stable path on Linux/macOS; do not inherit a long or untrusted TMPDIR.
    let path = PathBuf::from(format!("/tmp/ntnt-{}", uid()));
    private_directory(&path)?;
    Ok(path)
}

#[cfg(unix)]
fn validate_parent(path: &Path) -> std::io::Result<()> {
    validate_directory(path.parent().ok_or_else(|| invalid("missing parent"))?)
        .map_err(|e| endpoint_error(path, e))
}

/// Probe without consuming a stream listener's backlog. A live stream socket
/// rejects a datagram peer with EPROTOTYPE even before listen(). In particular,
/// macOS STREAM connect can return ECONNREFUSED merely because a backlog is full.
#[cfg(unix)]
fn probe_live_endpoint(path: &Path) -> std::io::Result<()> {
    let probe = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::DGRAM, None)?;
    probe.set_nonblocking(true)?;
    match probe.connect(&socket2::SockAddr::unix(path)?) {
        Err(e) if e.raw_os_error() == Some(libc::EPROTOTYPE) => Ok(()),
        result => result,
    }
}

/// Connect within one deadline, including temporary listen-backlog saturation.
/// No request bytes are sent until an actual connection has been established.
#[cfg(unix)]
pub fn connect_client(
    path: &Path,
    timeout: std::time::Duration,
) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::io::ErrorKind;
    use std::time::{Duration, Instant};
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| invalid("invalid connect timeout"))?;
    let address = socket2::SockAddr::unix(path)?;
    loop {
        connect_remaining(deadline)?;
        validate_client_endpoint(path)?;
        let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
        socket.set_nonblocking(true)?;
        let connected = match socket.connect(&address) {
            Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) => {
                wait_for_connect(&socket, deadline)
            }
            result => result,
        };
        match connected {
            Ok(()) => {
                socket.set_nonblocking(false)?;
                return Ok(socket.into());
            }
            Err(e)
                if e.kind() == ErrorKind::WouldBlock
                    || e.kind() == ErrorKind::Interrupted
                    || (e.kind() == ErrorKind::ConnectionRefused
                        && probe_live_endpoint(path).is_ok()) =>
            {
                // Linux EAGAIN does not initiate a pending AF_UNIX connection:
                // polling that unconnected fd can misleadingly report SO_ERROR=0.
                // Close it and retry, without ever replaying a command. On macOS,
                // retry ECONNREFUSED only when the independent probe proves live.
                drop(socket);
                std::thread::sleep(connect_remaining(deadline)?.min(Duration::from_millis(10)));
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(unix)]
fn connect_remaining(deadline: std::time::Instant) -> std::io::Result<std::time::Duration> {
    deadline
        .checked_duration_since(std::time::Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "control socket connect deadline elapsed",
            )
        })
}

#[cfg(unix)]
fn wait_for_connect(socket: &socket2::Socket, deadline: std::time::Instant) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    loop {
        let timeout = connect_remaining(deadline)?
            .as_millis()
            .clamp(1, i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        // A single valid descriptor and a bounded millisecond timeout.
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            continue;
        }
        if let Some(error) = socket.take_error()? {
            return Err(error);
        }
        // Confirm connection completion rather than treating writability alone
        // (including a hung-up/unconnected descriptor) as success.
        socket.peer_addr()?;
        return Ok(());
    }
}

/// Refuse symlinks, non-sockets, and endpoints not restricted to this user.
#[cfg(unix)]
pub fn validate_client_endpoint(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    validate_parent(path)?;
    let m = std::fs::symlink_metadata(path)?;
    if !m.file_type().is_socket() || m.uid() != uid() || m.mode() & 0o077 != 0 {
        return Err(endpoint_error(
            path,
            "endpoint must be an owned, owner-only socket (no symlinks)",
        ));
    }
    Ok(())
}

#[cfg(unix)]
static SOCKET_HANDLE: LazyLock<Mutex<Option<SocketHandle>>> = LazyLock::new(|| Mutex::new(None));

/// A staged listener: dropping on startup failure preserves the previous listener.
/// Commit only after all workers have been started successfully.
pub struct Startup {
    #[cfg(unix)]
    guard: std::sync::MutexGuard<'static, Option<SocketHandle>>,
    #[cfg(unix)]
    new: Option<SocketHandle>,
}
impl Startup {
    pub fn commit(self) {
        #[cfg(unix)]
        {
            let mut startup = self;
            if let Some(new) = startup.new.take() {
                new.ready.store(true, Ordering::Release);
                *startup.guard = Some(new);
            }
        }
    }
}

/// Acquire ownership and start listening before any workers run.
pub fn start_control_socket(options: &ControlOptions) -> std::io::Result<Startup> {
    #[cfg(not(unix))]
    {
        if options.control_socket.is_some()
            || options.worker_group.is_some()
            || std::env::var_os("NTNT_CONTROL_SOCKET").is_some()
            || std::env::var_os("NTNT_WORKER_GROUP").is_some()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Unix worker control sockets and worker groups are unsupported on Windows",
            ));
        }
        Ok(Startup {})
    }
    #[cfg(unix)]
    {
        let context = SOURCE
            .with(|s| s.borrow().clone())
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        let path = resolve(&context, options)?;
        let guard = SOCKET_HANDLE
            .lock()
            .map_err(|_| endpoint_error(&path, "listener mutex poisoned"))?;
        if guard.as_ref().is_some_and(|h| {
            h.path == path && h.owns_path() && !h.thread.as_ref().unwrap().is_finished()
        }) {
            return Ok(Startup { guard, new: None });
        }
        let new = SocketHandle::bind(&path).map_err(|e| endpoint_error(&path, e))?;
        Ok(Startup {
            guard,
            new: Some(new),
        })
    }
}

/// Stop accepting, join the listener, remove only its inode, then release the lock.
pub fn stop_control_socket() {
    #[cfg(unix)]
    if let Ok(mut guard) = SOCKET_HANDLE.lock() {
        *guard = None;
    }
}

#[cfg(unix)]
struct SocketHandle {
    ready: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    path: PathBuf,
    identity: (u64, u64),
    thread: Option<std::thread::JoinHandle<()>>,
    // Never unlink the sidecar: its stable inode is the cross-process lock identity.
    _lock: std::fs::File,
}

#[cfg(unix)]
impl SocketHandle {
    fn owns_path(&self) -> bool {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        std::fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.file_type().is_socket() && (m.dev(), m.ino()) == self.identity)
    }
    fn bind(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
        use std::os::unix::net::UnixListener;
        validate_parent(path)?;
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&lock_path)
            .map_err(|e| endpoint_error(Path::new(&lock_path), e))?;
        let m = lock.metadata()?;
        if !m.is_file() || m.uid() != uid() || m.mode() & 0o777 != 0o600 || m.nlink() != 1 {
            return Err(invalid(format!(
                "unsafe lock file {} (requires owned regular file, 0600, one link)",
                Path::new(&lock_path).display()
            )));
        }
        lock.try_lock().map_err(|e| {
            std::io::Error::other(format!(
                "cannot acquire ownership lock {}: {e}",
                Path::new(&lock_path).display()
            ))
        })?;
        match std::fs::symlink_metadata(path) {
            Ok(m) => {
                if !m.file_type().is_socket() || m.uid() != uid() || m.mode() & 0o077 != 0 {
                    return Err(invalid(
                        "existing endpoint is not an owned, owner-only socket",
                    ));
                }
                match probe_live_endpoint(path) {
                    Err(e) if e.raw_os_error() == Some(libc::ECONNREFUSED) => (),
                    Ok(()) => return Err(invalid("existing endpoint is live")),
                    Err(e) => {
                        return Err(std::io::Error::other(format!(
                            "existing endpoint is live or ambiguous: {e}"
                        )))
                    }
                }
                let current = std::fs::symlink_metadata(path)?;
                if (current.dev(), current.ino()) != (m.dev(), m.ino()) {
                    return Err(invalid("endpoint changed during stale probe"));
                }
                std::fs::remove_file(path)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(path)?;
        let m = std::fs::symlink_metadata(path)?;
        let mut handle = Self {
            ready: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            path: path.to_path_buf(),
            identity: (m.dev(), m.ino()),
            thread: None,
            _lock: lock,
        };
        let setup = (|| {
            if !handle.owns_path() {
                return Err(invalid("endpoint replaced during bind"));
            }
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)
        })();
        if let Err(e) = setup {
            // Close the actual listener before the handle cleans up/releases its lock.
            drop(listener);
            return Err(e);
        }
        let cancel = handle.cancel.clone();
        let ready = handle.ready.clone();
        handle.thread = Some(
            std::thread::Builder::new()
                .name("ntnt-control-socket".into())
                .spawn(move || run_accept_loop(listener, cancel, ready))?,
        );
        Ok(handle)
    }
}
#[cfg(unix)]
impl Drop for SocketHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if self.owns_path() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
fn run_accept_loop(
    listener: std::os::unix::net::UnixListener,
    cancel: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
) {
    while !cancel.load(Ordering::Acquire) {
        if !ready.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(10));
            continue;
        }
        match listener.accept() {
            Ok((stream, _)) => handle_connection(stream, &cancel),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(e) => {
                eprintln!("[ntnt] control socket accept: {e}");
                break;
            }
        }
    }
}

#[cfg(unix)]
fn handle_connection(mut stream: std::os::unix::net::UnixStream, cancel: &AtomicBool) {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut line = Vec::new();
    let mut buf = [0; 1024];
    while line.len() < 65_536 && !line.contains(&b'\n') {
        if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
            return;
        }
        let count = buf.len().min(65_536 - line.len());
        match stream.read(&mut buf[..count]) {
            Ok(0) => break,
            Ok(n) => line.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(_) => return,
        }
    }
    if line.is_empty() || cancel.load(Ordering::Acquire) {
        return;
    }
    let response = if let Some(end) = line.iter().position(|b| *b == b'\n') {
        match std::str::from_utf8(&line[..end]) {
            Ok(line) => dispatch_command(line.trim()),
            Err(_) => return,
        }
    } else if line.len() == 65_536 {
        serde_json::json!({"error": "request too large (max 64KB)"}).to_string()
    } else {
        match std::str::from_utf8(&line) {
            Ok(line) => dispatch_command(line.trim()),
            Err(_) => return,
        }
    };
    let response = format!("{response}\n");
    let mut remaining = response.as_bytes();
    while !remaining.is_empty() && !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
        match stream.write(remaining) {
            Ok(0) => break,
            Ok(n) => remaining = &remaining[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(_) => break,
        }
    }
}

// ── Command dispatch ──────────────────────────────────────────────────────────

fn dispatch_command(line: &str) -> String {
    let cmd: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({ "error": format!("invalid JSON: {}", e) }).to_string();
        }
    };

    match cmd.get("cmd").and_then(|v| v.as_str()) {
        Some("status") => cmd_status(),
        Some("scale") => {
            let band = match cmd.get("band").and_then(|v| v.as_str()) {
                Some(b) => b.to_string(),
                None => return serde_json::json!({ "error": "missing 'band' field" }).to_string(),
            };
            let count = match cmd.get("count").and_then(|v| v.as_u64()) {
                Some(c) if c >= 1 && c <= usize::MAX as u64 => c as usize,
                Some(c) if c > usize::MAX as u64 => {
                    return serde_json::json!({ "error": format!("count {} exceeds maximum ({})", c, usize::MAX) }).to_string()
                }
                Some(_) => return serde_json::json!({ "error": "count must be >= 1" }).to_string(),
                None => {
                    return serde_json::json!({ "error": "missing or invalid 'count' field" })
                        .to_string()
                }
            };
            cmd_scale(&band, count)
        }
        Some("pause") => match get_queue(&cmd) {
            Ok(q) => cmd_queue_paused(&q, true),
            Err(e) => e,
        },
        Some("resume") => match get_queue(&cmd) {
            Ok(q) => cmd_queue_paused(&q, false),
            Err(e) => e,
        },
        Some("batches") => {
            let status = cmd.get("status").and_then(|v| v.as_str());
            let limit = cmd
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .min(10_000) as usize;
            cmd_batches(status, limit)
        }
        Some("batch_status") => match cmd.get("batch_id").and_then(|v| v.as_str()) {
            Some(bid) => cmd_batch_status(bid),
            None => serde_json::json!({ "error": "missing 'batch_id' field" }).to_string(),
        },
        _ => serde_json::json!({ "error": "unknown command; expected 'status', 'scale', 'pause', 'resume', 'batches', or 'batch_status'" })
            .to_string(),
    }
}

fn cmd_status() -> String {
    match crate::stdlib::jobs::worker_status_impl() {
        Ok(value) => {
            let json = crate::stdlib::json::intent_value_to_json(&value);
            serde_json::to_string(&json)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
        }
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}

fn cmd_scale(band: &str, count: usize) -> String {
    match crate::stdlib::jobs::scale_workers_impl(band, count) {
        Ok(_) => serde_json::json!({ "ok": true, "band": band, "count": count }).to_string(),
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}

fn get_queue(cmd: &serde_json::Value) -> Result<String, String> {
    cmd.get("queue")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| serde_json::json!({ "error": "missing 'queue' field" }).to_string())
}

fn cmd_batches(status: Option<&str>, limit: usize) -> String {
    match crate::stdlib::jobs::list_batches_impl(status, limit) {
        Ok(value) => {
            let json = crate::stdlib::json::intent_value_to_json(&value);
            serde_json::to_string(&json)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
        }
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}

fn cmd_batch_status(batch_id: &str) -> String {
    match crate::stdlib::jobs::batch_status_impl(batch_id) {
        Ok(value) => {
            let json = crate::stdlib::json::intent_value_to_json(&value);
            serde_json::to_string(&json)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
        }
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}

fn cmd_queue_paused(queue: &str, paused: bool) -> String {
    let result = if paused {
        crate::stdlib::jobs::pause_queue_impl(queue)
    } else {
        crate::stdlib::jobs::resume_queue_impl(queue)
    };
    match result {
        Ok(_) => serde_json::json!({ "ok": true, "queue": queue, "paused": paused }).to_string(),
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_invalid_json() {
        let resp = dispatch_command("not json");
        assert!(resp.contains("invalid JSON"));
    }

    #[test]
    fn test_dispatch_unknown_command() {
        let resp = dispatch_command(r#"{"cmd":"reboot"}"#);
        assert!(resp.contains("unknown command"));
    }

    #[test]
    fn test_dispatch_pause_missing_queue() {
        let resp = dispatch_command(r#"{"cmd":"pause"}"#);
        assert!(resp.contains("missing 'queue'"));
    }

    #[test]
    fn test_dispatch_resume_missing_queue() {
        let resp = dispatch_command(r#"{"cmd":"resume"}"#);
        assert!(resp.contains("missing 'queue'"));
    }

    #[test]
    fn test_dispatch_scale_missing_band() {
        let resp = dispatch_command(r#"{"cmd":"scale","count":4}"#);
        assert!(resp.contains("missing 'band'"));
    }

    #[test]
    fn test_dispatch_scale_missing_count() {
        let resp = dispatch_command(r#"{"cmd":"scale","band":"low"}"#);
        assert!(resp.contains("missing or invalid 'count'"));
    }

    #[test]
    fn test_dispatch_scale_zero_count() {
        let resp = dispatch_command(r#"{"cmd":"scale","band":"low","count":0}"#);
        assert!(resp.contains("count must be >= 1"));
    }

    #[test]
    fn test_dispatch_no_cmd_field() {
        let resp = dispatch_command(r#"{"band":"low"}"#);
        assert!(resp.contains("unknown command"));
    }
}

#[cfg(all(test, unix))]
mod ownership_tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    fn directory() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        d
    }
    #[test]
    fn client_deadline_and_stale_failure_are_distinct() {
        let d = directory();
        let path = d.path().join("pending.sock");
        let socket =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        socket
            .bind(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let timeout = std::time::Duration::from_millis(50);
        let start = std::time::Instant::now();
        assert_eq!(
            connect_client(&path, timeout).unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() >= timeout);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        drop(socket);
        assert_eq!(
            connect_client(&path, timeout).unwrap_err().kind(),
            std::io::ErrorKind::ConnectionRefused
        );
    }

    #[test]
    fn bound_socket_before_listen_is_not_stale() {
        let d = directory();
        let path = d.path().join("binding.sock");
        let socket =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        socket
            .bind(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        assert!(
            SocketHandle::bind(&path).is_err(),
            "a bound socket is live even before listen"
        );
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
    }

    #[test]
    fn failed_reconfiguration_and_uncommitted_start_preserve_listener() {
        let d = directory();
        let path = d.path().join("first.sock");
        let options = ControlOptions {
            control_socket: Some(path.clone()),
            worker_group: None,
        };
        start_control_socket(&options).unwrap().commit();
        let inode = std::fs::metadata(&path).unwrap().ino();
        start_control_socket(&options).unwrap().commit();
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        let other = d.path().join("other.sock");
        let options = ControlOptions {
            control_socket: Some(other.clone()),
            worker_group: None,
        };
        std::fs::write(&other, "preserve").unwrap();
        assert!(start_control_socket(&options).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        std::fs::remove_file(&other).unwrap();
        drop(start_control_socket(&options).unwrap());
        assert!(!other.exists());
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        stop_control_socket();
        assert!(!path.exists());
    }
    #[test]
    fn private_runtime_paths_reject_symlinks_files_and_unsafe_modes() {
        let d = directory();
        let path = d.path().join("runtime");
        std::fs::write(&path, "untouched").unwrap();
        assert!(private_directory(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "untouched");
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(d.path(), &path).unwrap();
        assert!(private_directory(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(private_directory(&path).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o777);
    }
    #[test]
    fn canonical_source_identity_uses_nearest_manifest() {
        let d = directory();
        let nested = d.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let source = nested.join("worker.tnt");
        std::fs::write(&source, "").unwrap();
        assert_eq!(
            project_identity(&source).unwrap(),
            nested.canonicalize().unwrap()
        );
        std::fs::write(d.path().join("ntnt.toml"), "").unwrap();
        assert_eq!(
            project_identity(&source).unwrap(),
            d.path().canonicalize().unwrap()
        );
        std::fs::write(nested.join("ntnt.toml"), "").unwrap();
        assert_eq!(
            project_identity(&source).unwrap(),
            nested.canonicalize().unwrap()
        );
    }
    #[test]
    fn hard_linked_lock_is_rejected() {
        let d = directory();
        let path = d.path().join("socket");
        let lock = d.path().join("socket.lock");
        std::fs::write(&lock, "preserve").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::hard_link(&lock, d.path().join("alias")).unwrap();
        assert!(SocketHandle::bind(&path).is_err());
        assert_eq!(std::fs::read_to_string(&lock).unwrap(), "preserve");
    }
}
