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

    // Remove stale socket file from a previous run.
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

    // Non-blocking accept so the loop can check the cancellation flag.
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("[ntnt] control socket: set_nonblocking failed: {}", e);
        return;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = Arc::clone(&cancel);

    std::thread::Builder::new()
        .name("ntnt-control-socket".to_string())
        .spawn(move || run_accept_loop(listener, cancel_thread))
        .ok();

    let handle = SocketHandle { cancel, path };
    if let Ok(mut guard) = SOCKET_HANDLE.lock() {
        *guard = Some(handle);
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
    use std::io::{BufRead, BufReader, Write};

    // Switch back to blocking for this individual connection.
    let _ = stream.set_nonblocking(false);

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();

    if reader.read_line(&mut line).is_err() {
        return;
    }

    let response = dispatch_command(line.trim());

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
                Some(c) if c >= 1 => c as usize,
                Some(_) => return serde_json::json!({ "error": "count must be >= 1" }).to_string(),
                None => {
                    return serde_json::json!({ "error": "missing or invalid 'count' field" })
                        .to_string()
                }
            };
            cmd_scale(&band, count)
        }
        _ => serde_json::json!({ "error": "unknown command; expected 'status' or 'scale'" })
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
