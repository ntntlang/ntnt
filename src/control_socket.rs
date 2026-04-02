//! Unix domain socket control plane for ntnt job workers.
//!
//! Provides a JSON command interface for live worker management over a Unix
//! domain socket at `.ntnt.sock` in the current working directory.
//!
//! ## Protocol
//!
//! Newline-delimited JSON: send one JSON object + newline, receive one JSON
//! object + newline, then the connection is closed.
//!
//! Commands:
//! - `{"cmd": "status"}` → worker status snapshot (same shape as worker_status())
//! - `{"cmd": "scale", "band": "low", "count": 8}` → scale a worker band
//!
//! ## Example
//!
//! ```bash
//! echo '{"cmd":"status"}' | socat - UNIX-CONNECT:.ntnt.sock
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

// ── Global handle ────────────────────────────────────────────────────────────

/// Globally held socket handle — dropping it cancels the accept thread and
/// removes the socket file.  Replaced each time start_control_socket() is called.
static SOCKET_HANDLE: LazyLock<Mutex<Option<SocketHandle>>> = LazyLock::new(|| Mutex::new(None));

struct SocketHandle {
    cancel: Arc<AtomicBool>,
    path: std::path::PathBuf,
}

impl Drop for SocketHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        let _ = std::fs::remove_file(&self.path);
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Start the control socket in a background thread.
///
/// On Unix: binds `.ntnt.sock` in the current working directory, removing any
/// stale socket file first.  Stores the handle globally — calling this a second
/// time replaces the previous socket (the old handle is dropped, removing its
/// socket file and stopping its thread).
///
/// On Windows: logs a message and returns immediately.
pub fn start_control_socket() {
    #[cfg(unix)]
    start_unix();

    #[cfg(not(unix))]
    {
        eprintln!("[ntnt] control socket is not available on Windows");
    }
}

/// Stop the control socket (cancel the accept thread, remove the socket file).
pub fn stop_control_socket() {
    if let Ok(mut guard) = SOCKET_HANDLE.lock() {
        *guard = None;
    }
}

// ── Unix implementation ───────────────────────────────────────────────────────

#[cfg(unix)]
fn start_unix() {
    use std::os::unix::net::UnixListener;

    let path = match std::env::current_dir() {
        Ok(d) => d.join(".ntnt.sock"),
        Err(e) => {
            eprintln!("[ntnt] control socket: failed to get CWD: {}", e);
            return;
        }
    };

    // Drop the old handle BEFORE binding — SocketHandle::drop removes the socket
    // file, so dropping after bind would unlink the newly-bound socket.
    if let Ok(mut guard) = SOCKET_HANDLE.lock() {
        *guard = None;
    }

    // Remove stale socket file from a previous run (e.g., crash without cleanup).
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[ntnt] control socket: failed to bind {}: {}",
                path.display(),
                e
            );
            return;
        }
    };

    // Restrict socket permissions to owner only (0600) immediately after bind.
    // If this fails, remove the socket and abort — don't leave a permissive socket.
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            eprintln!(
                "[ntnt] control socket: failed to set permissions on {}: {} — removing socket",
                path.display(),
                e
            );
            let _ = std::fs::remove_file(&path);
            return;
        }
    }

    // Non-blocking accept so the loop can check the cancellation flag.
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("[ntnt] control socket: set_nonblocking failed: {}", e);
        let _ = std::fs::remove_file(&path);
        return;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = Arc::clone(&cancel);

    match std::thread::Builder::new()
        .name("ntnt-control-socket".to_string())
        .spawn(move || run_accept_loop(listener, cancel_thread))
    {
        Ok(_) => {
            let handle = SocketHandle { cancel, path };
            if let Ok(mut guard) = SOCKET_HANDLE.lock() {
                *guard = Some(handle);
            }
        }
        Err(e) => {
            eprintln!("[ntnt] control socket: failed to spawn thread: {}", e);
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(unix)]
fn run_accept_loop(listener: std::os::unix::net::UnixListener, cancel: Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }

        match listener.accept() {
            Ok((stream, _)) => handle_connection(stream),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                continue; // EINTR from signal delivery — retry accept
            }
            Err(e) => {
                if !cancel.load(Ordering::Acquire) {
                    eprintln!("[ntnt] control socket: accept error: {}", e);
                }
                break;
            }
        }
    }
}

#[cfg(unix)]
fn handle_connection(stream: std::os::unix::net::UnixStream) {
    use std::io::{BufRead, BufReader, Read, Write};

    // Switch to blocking with a 5-second timeout — prevents a misbehaving
    // client from blocking the control socket thread indefinitely.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));

    // Enforce 64KB max request size at the Read level — take() limits how many
    // bytes BufReader can read, preventing large allocations before the newline check.
    const MAX_REQUEST_SIZE: u64 = 65_536;
    let limited = (&stream).take(MAX_REQUEST_SIZE);
    let mut reader = BufReader::new(limited);
    let mut line = String::new();

    match reader.read_line(&mut line) {
        Ok(0) => return,
        Err(_) => return,
        _ => {}
    }

    // Detect truncation: max bytes reached without a newline
    let response = if line.len() as u64 >= MAX_REQUEST_SIZE && !line.ends_with('\n') {
        serde_json::json!({ "error": format!("request too large (max {}KB)", MAX_REQUEST_SIZE / 1024) }).to_string()
    } else {
        dispatch_command(line.trim())
    };

    let mut writer = std::io::BufWriter::new(&stream);
    let _ = writeln!(writer, "{}", response);
    let _ = writer.flush();
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
            let limit = cmd.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
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
