//! Standard Library Modules for Intent
//!
//! Each module provides functions that can be imported into Intent programs:
//! ```intent
//! import { split, join } from "std/string"
//! import { sin, cos, PI } from "std/math"
//! ```

pub mod auth;
pub mod client_ip;
pub mod collections;
pub mod concurrent;
pub mod crypto;
pub mod csv;
pub mod email;
pub mod env;
pub mod fs;
pub mod http;
pub mod http_bridge;
pub mod http_server;
pub mod http_server_async;
pub mod jobs;
pub mod json;
pub mod kv;
pub mod log;
pub mod markdown;
pub mod math;
pub mod net;
pub mod netmon;
pub mod path;
pub mod postgres;
pub mod process;
pub mod rate_limit;
pub mod realtime;
pub mod secrets;
pub(crate) mod send_deadline;
pub mod sqlite;
pub mod string;
pub mod template;
pub mod time;
pub(crate) mod tls_trust;
pub mod url;
pub mod validate;
pub mod websocket;

use crate::interpreter::Value;
use std::collections::HashMap;

/// Type alias for stdlib module initialization functions
pub type StdlibModule = HashMap<String, Value>;

/// Capture host-owned runtime security policy (`std/process` execution and the
/// `fetch()` SSRF policy) from the real process environment, together with the
/// names of the variables the host set, which `load_env` never replaces.
///
/// The first call in a process wins and later calls are no-ops, so environment
/// changes made by application code (for example `load_env`) cannot grant
/// capabilities. `main()` and `Interpreter::new()` both call this before any
/// source is evaluated: every CLI command, server/job/task worker interpreter
/// and embedding of the public `Interpreter` API constructs an interpreter
/// first. Policy consumers also capture lazily for direct Rust callers.
pub fn capture_host_policy() {
    process::capture_host_policy();
    http::capture_host_policy();
    env::capture_host_policy();
}

/// Stop process-local runtimes before the host exits.
pub fn shutdown_runtimes() {
    crate::control_socket::stop_control_socket();
    net::persistent::shutdown();
    realtime::shutdown();
    concurrent::RUNTIME.shutdown();
    process::RUNTIME.shutdown();
    if let Err(e) = fs::owned::shutdown() {
        eprintln!("temporary resource shutdown failed: {e}");
    }
    if let Err(e) = net::tcp::shutdown() {
        eprintln!("TCP shutdown failed: {e}");
    }
}

/// Initialize all standard library modules
pub fn init_all_modules() -> HashMap<String, StdlibModule> {
    let mut modules = HashMap::new();

    modules.insert("std/string".to_string(), string::init());
    modules.insert("std/math".to_string(), math::init());
    modules.insert("std/collections".to_string(), collections::init());
    modules.insert("std/env".to_string(), env::init());
    modules.insert("std/secrets".to_string(), secrets::init());
    modules.insert("std/fs".to_string(), fs::init());
    modules.insert("std/path".to_string(), path::init());
    modules.insert("std/json".to_string(), json::init());
    modules.insert("std/time".to_string(), time::init());
    modules.insert("std/crypto".to_string(), crypto::init());
    modules.insert("std/url".to_string(), url::init());
    modules.insert("std/net".to_string(), net::init());
    modules.insert("std/netmon".to_string(), netmon::init());
    modules.insert("std/http".to_string(), http::init());
    modules.insert("std/http/server".to_string(), http_server::init());
    modules.insert("std/db/postgres".to_string(), postgres::init());
    modules.insert("std/db/sqlite".to_string(), sqlite::init());
    modules.insert("std/concurrent".to_string(), concurrent::init());
    modules.insert("std/csv".to_string(), csv::init());
    modules.insert("std/template".to_string(), template::init());
    modules.insert("std/log".to_string(), log::init());
    modules.insert("std/markdown".to_string(), markdown::init());
    modules.insert("std/process".to_string(), process::init());
    modules.insert("std/kv".to_string(), kv::create_kv_module());
    modules.insert("std/jobs".to_string(), jobs::init());
    modules.insert("std/auth".to_string(), auth::init());
    modules.insert("std/validate".to_string(), validate::init());
    modules.insert("std/email".to_string(), email::init());

    modules
}
