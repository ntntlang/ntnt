//! std/env module - Environment variable access

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

/// Host runtime-policy keys that application code may not set. The runtime
/// reads them from the environment that starts ntnt (process execution and
/// `fetch()` SSRF policy are captured before any source is evaluated), so a
/// dotenv file loaded by the application must not grant or relax them.
/// `NTNT_ENV` is application configuration and stays loadable.
const RESERVED_POLICY_KEYS: &[&str] = &[
    "NTNT_PROCESS_ENABLE",
    "NTNT_PROCESS_ALLOW",
    "NTNT_SSRF_PROTECTION",
    "NTNT_ALLOW_LOCALHOST",
    "NTNT_ALLOW_PRIVATE_IPS",
    "NTNT_BLOCKED_HOSTS",
    "NTNT_NET_ALLOW_PRIVATE",
    "NTNT_NETMON_ENABLE",
];

/// Every `std/secrets` provider-selection and provider-configuration key.
const RESERVED_POLICY_PREFIX: &str = "NTNT_SECRETS_";

/// Environment names are case-insensitive on Windows, so reserved names are
/// matched case-insensitively on every platform.
fn is_reserved_policy_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.starts_with(RESERVED_POLICY_PREFIX)
        || RESERVED_POLICY_KEYS
            .iter()
            .any(|reserved| *reserved == upper)
}

/// Warns once per process for each reserved key that `load_env` skipped.
fn warn_skipped_policy_key(key: &str) {
    static WARNED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);
    let first = WARNED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(key.to_ascii_uppercase());
    if first {
        eprintln!(
            "[WARN] load_env: ignored {key}; runtime security policy is read from the environment that starts ntnt and cannot be changed by application code"
        );
    }
}

/// Initialize the std/env module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt get_env
    // @module std/env
    // @module_description Environment variable access
    // @signature get_env(name: String) -> Option<String>
    // Gets an environment variable by name.
    //
    // Returns Some(value) if the variable is set, or None if it is not defined
    // in the current process environment. This API is independent of optional
    // `std/secrets` providers and remains available when `NTNT_ENV=production`.
    // @param name The environment variable name
    // @returns Option containing the value or None
    // @see_also load_env
    // @since v0.1.0
    // @example get_env("HOME") => Some("/Users/...") ~ "Get home directory"
    // @example get_env("UNDEFINED_VAR") => None ~ "Missing variable"
    module.insert(
        "get_env".to_string(),
        Value::NativeFunction {
            name: "get_env".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(name) => match std::env::var(name) {
                    Ok(val) => Ok(Value::some(Value::String(val))),
                    Err(_) => Ok(Value::none()),
                },
                _ => Err(IntentError::type_error(
                    "get_env() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt args
    // @module std/env
    // @signature args() -> Array<String>
    // Returns command-line arguments as an array of strings.
    //
    // The first element is the program name, followed by any arguments
    // passed on the command line.
    // @returns Array of argument strings
    // @since v0.1.0
    // @example args() => ["ntnt", "script.tnt", "--flag"] ~ "Program name and arguments"
    module.insert(
        "args".to_string(),
        Value::NativeFunction {
            name: "args".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| {
                let args: Vec<Value> = std::env::args().map(Value::String).collect();
                Ok(Value::Array(args))
            },
        },
    );

    // @ntnt cwd
    // @module std/env
    // @signature cwd() -> String
    // Returns the current working directory as a string.
    // @returns Absolute path of the current working directory
    // @since v0.1.0
    // @tags #deterministic
    // @example cwd() => "/Users/dev/project" ~ "Current directory path"
    // @error RuntimeError ~ "Failed to get cwd" fix: "Ensure the process has filesystem access"
    module.insert(
        "cwd".to_string(),
        Value::NativeFunction {
            name: "cwd".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| match std::env::current_dir() {
                Ok(path) => Ok(Value::String(path.to_string_lossy().to_string())),
                Err(e) => Err(IntentError::runtime_error(format!(
                    "Failed to get cwd: {}",
                    e
                ))),
            },
        },
    );

    // @ntnt load_env
    // @module std/env
    // @signature load_env(path: String) -> Result<Unit, String>
    // Loads environment variables from a .env file.
    //
    // Reads the file line by line, parsing KEY=VALUE pairs. Lines starting
    // with # are treated as comments and skipped. Variables are set in
    // the current process environment. Loading `.env` files remains supported
    // in production and is not replaced or restricted by optional `std/secrets`.
    //
    // Host runtime-policy keys are reserved and skipped with one stderr warning
    // per key, while the rest of the file still loads: NTNT_PROCESS_ENABLE,
    // NTNT_PROCESS_ALLOW, NTNT_SSRF_PROTECTION, NTNT_ALLOW_LOCALHOST,
    // NTNT_ALLOW_PRIVATE_IPS, NTNT_BLOCKED_HOSTS, NTNT_NET_ALLOW_PRIVATE,
    // NTNT_NETMON_ENABLE, and every NTNT_SECRETS_* key (matched
    // case-insensitively). Set them in the environment that starts ntnt.
    // NTNT_ENV is not reserved.
    // @param path Path to the .env file
    // @returns Result indicating success or file read error
    // @see_also get_env
    // @since v0.2.0
    // @example load_env(".env") ~ "Load default .env file"
    // @example load_env(".env.local") ~ "Load environment-specific file"
    // @gotcha Reserved runtime-policy keys in the file are ignored with a warning; process execution and fetch() SSRF policy come from the environment that starts ntnt
    module.insert(
        "load_env".to_string(),
        Value::NativeFunction {
            name: "load_env".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                match &args[0] {
                    Value::String(path) => {
                        match std::fs::read_to_string(path) {
                            Ok(content) => {
                                for line in content.lines() {
                                    let line = line.trim();
                                    // Skip comments and empty lines
                                    if line.is_empty() || line.starts_with('#') {
                                        continue;
                                    }
                                    // Parse KEY=VALUE
                                    if let Some((key, value)) = line.split_once('=') {
                                        let key = key.trim();
                                        if is_reserved_policy_key(key) {
                                            warn_skipped_policy_key(key);
                                            continue;
                                        }
                                        std::env::set_var(key, value.trim());
                                    }
                                }
                                Ok(Value::ok(Value::Unit))
                            }
                            Err(e) => Ok(Value::err(Value::String(format!(
                                "Failed to load .env: {}",
                                e
                            )))),
                        }
                    }
                    _ => Err(IntentError::type_error(
                        "load_env() requires a string path".to_string(),
                    )),
                }
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_load_env(path: &std::path::Path) -> Value {
        let module = init();
        let Some(Value::NativeFunction { func, .. }) = module.get("load_env") else {
            panic!("std/env exports load_env");
        };
        func(&[Value::String(path.to_string_lossy().into_owned())]).expect("load_env result")
    }

    #[test]
    fn load_env_skips_reserved_runtime_policy_keys_and_loads_the_rest() {
        let reserved = [
            "NTNT_PROCESS_ENABLE",
            "NTNT_PROCESS_ALLOW",
            "NTNT_SSRF_PROTECTION",
            "NTNT_ALLOW_LOCALHOST",
            "NTNT_ALLOW_PRIVATE_IPS",
            "NTNT_BLOCKED_HOSTS",
            "NTNT_NET_ALLOW_PRIVATE",
            "NTNT_NETMON_ENABLE",
            "NTNT_SECRETS_PROVIDER",
            "NTNT_SECRETS_SOCKET_ENDPOINTS",
            "NTNT_SECRETS_AUTHORIZATION_SCOPE",
            "NTNT_SECRETS_TIMEOUT_MS",
        ];
        // Windows environment names are case-insensitive, so spelling variants
        // of a reserved key are reserved on every platform.
        let variant = "Ntnt_Process_Enable";
        let before: Vec<_> = reserved
            .iter()
            .chain([&variant])
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        let directory = tempfile::tempdir().unwrap();
        let dotenv = directory.path().join(".env");
        let mut contents = String::from("# application configuration\n");
        for key in reserved.iter().chain([&variant]) {
            contents.push_str(&format!("{key}=from-dotenv\n"));
        }
        contents.push_str("STD_ENV_LOAD_ENV_ORDINARY_KEY = loaded \n");
        std::fs::write(&dotenv, contents).unwrap();

        let result = call_load_env(&dotenv);

        assert!(
            matches!(&result, Value::EnumValue { enum_name, variant, .. } if enum_name == "Result" && variant == "Ok"),
            "load_env must keep loading the file: {result:?}"
        );
        for (key, previous) in before {
            assert_eq!(std::env::var_os(key), previous, "load_env changed {key}");
        }
        assert_eq!(
            std::env::var("STD_ENV_LOAD_ENV_ORDINARY_KEY").as_deref(),
            Ok("loaded")
        );
    }
}
