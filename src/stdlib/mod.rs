//! Standard Library Modules for Intent
//!
//! Each module provides functions that can be imported into Intent programs:
//! ```intent
//! import { split, join } from "std/string"
//! import { sin, cos, PI } from "std/math"
//! ```

pub mod collections;
pub mod concurrent;
pub mod crypto;
pub mod csv;
pub mod env;
pub mod fs;
pub mod http;
pub mod http_bridge;
pub mod http_server;
pub mod http_server_async;
pub mod json;
pub mod math;
pub mod path;
pub mod postgres;
pub mod sqlite;
pub mod string;
pub mod template;
pub mod time;
pub mod url;

use crate::interpreter::Value;
use std::collections::HashMap;

/// Type alias for stdlib module initialization functions
pub type StdlibModule = HashMap<String, Value>;

/// Initialize all standard library modules
pub fn init_all_modules() -> HashMap<String, StdlibModule> {
    let mut modules = HashMap::new();

    modules.insert("std/string".to_string(), string::init());
    modules.insert("std/math".to_string(), math::init());
    modules.insert("std/collections".to_string(), collections::init());
    modules.insert("std/env".to_string(), env::init());
    modules.insert("std/fs".to_string(), fs::init());
    modules.insert("std/path".to_string(), path::init());
    modules.insert("std/json".to_string(), json::init());
    modules.insert("std/time".to_string(), time::init());
    modules.insert("std/crypto".to_string(), crypto::init());
    modules.insert("std/url".to_string(), url::init());
    modules.insert("std/http".to_string(), http::init());
    modules.insert("std/http/server".to_string(), http_server::init());
    modules.insert("std/db/postgres".to_string(), postgres::init());
    modules.insert("std/db/sqlite".to_string(), sqlite::init());
    modules.insert("std/concurrent".to_string(), concurrent::init());
    modules.insert("std/csv".to_string(), csv::init());
    modules.insert("std/template".to_string(), template::init());

    modules
}
