//! Provider-neutral secret lookup for ntnt applications.

use crate::error::{IntentError, Result};
use crate::interpreter::{SecretValue, Value};
use crate::secret::validate_secret_name;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use zeroize::Zeroizing;

#[cfg(unix)]
#[path = "secrets/socket.rs"]
mod socket;

const PROVIDER_ENV: &str = "NTNT_SECRETS_PROVIDER";
const SOCKET_ENDPOINTS_ENV: &str = "NTNT_SECRETS_SOCKET_ENDPOINTS";
const SOCKET_SCOPE_ENV: &str = "NTNT_SECRETS_AUTHORIZATION_SCOPE";
const SOCKET_TIMEOUT_ENV: &str = "NTNT_SECRETS_TIMEOUT_MS";
const DEFAULT_ATTEMPTS_PER_ENDPOINT: usize = 2;
const DEFAULT_SOCKET_TIMEOUT_MS: u64 = 1_000;
const MIN_SOCKET_TIMEOUT_MS: u64 = 10;
const MAX_SOCKET_TIMEOUT_MS: u64 = 10_000;
const MAX_SOCKET_ENDPOINTS: usize = 8;
const MAX_SOCKET_PATH_BYTES: usize = 96;
const MAX_AUTHORIZATION_SCOPE_BYTES: usize = 128;
const PRODUCTION_SOCKET_ROOT: &str = "/run/larri-secrets";

#[cfg_attr(test, allow(dead_code))]
#[derive(Default)]
enum SecretDeclarations {
    #[default]
    Unconfigured,
    NoManifest,
    Loaded(HashSet<String>),
    Invalid,
    ConflictingApplication,
}

#[cfg_attr(test, allow(dead_code))]
#[derive(Default)]
struct DeclarationState {
    identity: Option<PathBuf>,
    declarations: SecretDeclarations,
}

static SECRET_DECLARATIONS: OnceLock<RwLock<DeclarationState>> = OnceLock::new();

fn declaration_state() -> &'static RwLock<DeclarationState> {
    SECRET_DECLARATIONS.get_or_init(|| RwLock::new(DeclarationState::default()))
}

/// Configure manifest-backed secret declarations for an application's main source file.
///
/// The closest ancestor `ntnt.toml` wins. A project with a manifest must declare
/// every accessible secret under `[secrets.<NAME>]`; manifest-free development
/// remains compatible with simple environment-provider experiments.
/// ntnt runs one application per process. Reconfiguring a process for a different
/// application fails closed instead of letting the last interpreter replace policy.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn configure_for_source(source_path: &str) {
    let (identity, declarations) = load_declarations(Path::new(source_path));
    let lock = declaration_state();
    let mut state = match lock.write() {
        Ok(state) => state,
        Err(poisoned) => {
            // Recover the guard only to record a durable fail-closed state. Clearing
            // the poison makes later lookups report the explicit manifest error.
            let mut state = poisoned.into_inner();
            state.declarations = SecretDeclarations::Invalid;
            drop(state);
            lock.clear_poison();
            return;
        }
    };

    if matches!(
        state.declarations,
        SecretDeclarations::ConflictingApplication
    ) {
        return;
    }
    match &state.identity {
        None => {
            state.identity = Some(identity);
            state.declarations = declarations;
        }
        Some(current) if current == &identity => state.declarations = declarations,
        // A mixed application identity is a process-level security violation.
        // Keep the state permanently poisoned; only a process restart may reset it.
        Some(_) => state.declarations = SecretDeclarations::ConflictingApplication,
    }
}

#[cfg_attr(test, allow(dead_code))]
fn load_declarations(source_path: &Path) -> (PathBuf, SecretDeclarations) {
    let start = if source_path.is_file() {
        source_path.parent().unwrap_or(source_path)
    } else {
        source_path
    };

    let source_identity = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let manifest = start
        .ancestors()
        .map(|directory| directory.join("ntnt.toml"))
        .find(|candidate| candidate.is_file());
    let Some(manifest) = manifest else {
        return (source_identity, SecretDeclarations::NoManifest);
    };

    let Ok(content) = std::fs::read_to_string(manifest) else {
        return (source_identity, SecretDeclarations::Invalid);
    };
    let Ok(document) = content.parse::<toml::Value>() else {
        return (source_identity, SecretDeclarations::Invalid);
    };
    let Some(secrets) = document.get("secrets") else {
        return (source_identity, SecretDeclarations::Loaded(HashSet::new()));
    };
    let Some(table) = secrets.as_table() else {
        return (source_identity, SecretDeclarations::Invalid);
    };

    let mut declared = HashSet::with_capacity(table.len());
    for (name, metadata) in table {
        let Some(metadata) = metadata.as_table() else {
            return (source_identity, SecretDeclarations::Invalid);
        };
        if validate_secret_name(name).is_err()
            || metadata.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "label" | "description" | "required" | "environments"
                )
            })
            || metadata.get("label").is_some_and(|value| !value.is_str())
            || metadata
                .get("description")
                .is_some_and(|value| !value.is_str())
            || metadata
                .get("required")
                .is_some_and(|value| !value.is_bool())
            || metadata.get("environments").is_some_and(|value| {
                value
                    .as_array()
                    .is_none_or(|items| items.iter().any(|item| !item.is_str()))
            })
        {
            return (source_identity, SecretDeclarations::Invalid);
        }
        declared.insert(name.clone());
    }
    (source_identity, SecretDeclarations::Loaded(declared))
}

fn enforce_declared(name: &str) -> Result<()> {
    let declarations = declaration_state().read().map_err(|_| {
        IntentError::runtime_error("Secret declaration state is unavailable".to_string())
    })?;
    match &declarations.declarations {
        SecretDeclarations::Unconfigured | SecretDeclarations::NoManifest => Ok(()),
        SecretDeclarations::Loaded(names) if names.contains(name) => Ok(()),
        SecretDeclarations::Loaded(_) => Err(IntentError::runtime_error(format!(
            "Secret '{name}' is not declared in ntnt.toml"
        ))),
        SecretDeclarations::Invalid => Err(IntentError::runtime_error(
            "Secret declarations in ntnt.toml are invalid".to_string(),
        )),
        SecretDeclarations::ConflictingApplication => Err(IntentError::runtime_error(
            "Secret declarations cannot be shared across multiple applications in one process"
                .to_string(),
        )),
    }
}

/// A provider result that is deliberately not `Debug`: the found payload is plaintext.
enum ProviderLookup {
    Found(Zeroizing<String>),
    Missing,
}

/// A non-sensitive provider identity safe for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderEndpointLabel(String);

impl ProviderEndpointLabel {
    fn env() -> Self {
        Self("env".to_string())
    }

    #[cfg(unix)]
    fn socket(index: usize) -> Self {
        Self(format!("larri-socket-{index}"))
    }

    #[cfg(test)]
    fn fixture(value: &str) -> Self {
        assert!(value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-'));
        Self(value.to_string())
    }
}

impl fmt::Display for ProviderEndpointLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Stable failure classes for HA failover. The v0.5.1 environment provider
/// can only emit invalid configuration; socket providers use the remaining classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderErrorKind {
    Unavailable,
    AccessDenied,
    InvalidRequest,
    InvalidConfiguration,
}

#[derive(Debug, Clone)]
struct ProviderError {
    kind: ProviderErrorKind,
    endpoint: ProviderEndpointLabel,
}

impl ProviderError {
    fn new(kind: ProviderErrorKind, endpoint: &ProviderEndpointLabel) -> Self {
        Self {
            kind,
            endpoint: endpoint.clone(),
        }
    }
}

trait SecretProvider: Send + Sync {
    fn endpoint(&self) -> &ProviderEndpointLabel;
    fn authorization_scope(&self) -> &str;

    /// Perform one endpoint lookup. Implementations must apply a bounded timeout
    /// and classify timeouts/transient transport failures as `Unavailable`.
    fn lookup(&self, name: &str) -> std::result::Result<ProviderLookup, ProviderError>;
}

/// Ordered equivalent provider endpoints with classified failover.
///
/// Only `Unavailable` advances to the next endpoint. Missing, denied, invalid
/// request, and invalid configuration results are terminal so failover cannot
/// change authorization scope.
struct ProviderGroup {
    providers: Vec<Arc<dyn SecretProvider>>,
    attempts_per_endpoint: usize,
}

impl ProviderGroup {
    fn new(providers: Vec<Arc<dyn SecretProvider>>) -> Result<Self> {
        if providers.is_empty() {
            return Err(IntentError::runtime_error(
                "Secrets provider configuration has no endpoints".to_string(),
            ));
        }
        let scope = providers[0].authorization_scope();
        if scope.is_empty()
            || providers
                .iter()
                .any(|provider| provider.authorization_scope() != scope)
        {
            return Err(IntentError::runtime_error(
                "Secrets provider endpoints must share one authorization scope".to_string(),
            ));
        }
        Ok(Self {
            providers,
            attempts_per_endpoint: DEFAULT_ATTEMPTS_PER_ENDPOINT,
        })
    }

    fn lookup(&self, name: &str) -> Result<Option<SecretValue>> {
        let mut unavailable = Vec::new();
        let mut attempts = 0;

        for provider in &self.providers {
            for attempt in 0..self.attempts_per_endpoint {
                attempts += 1;
                match provider.lookup(name) {
                    Ok(ProviderLookup::Found(value)) => {
                        return SecretValue::new_zeroizing(name, value).map(Some)
                    }
                    Ok(ProviderLookup::Missing) => return Ok(None),
                    Err(error) => match error.kind {
                        ProviderErrorKind::Unavailable
                            if attempt + 1 < self.attempts_per_endpoint =>
                        {
                            continue;
                        }
                        ProviderErrorKind::Unavailable => {
                            unavailable.push(error.endpoint);
                            break;
                        }
                        ProviderErrorKind::AccessDenied => {
                            return Err(IntentError::runtime_error(format!(
                                "Secret provider access denied for '{name}' at endpoint '{}'",
                                error.endpoint
                            )))
                        }
                        ProviderErrorKind::InvalidRequest => {
                            return Err(IntentError::runtime_error(format!(
                                "Secret provider rejected the request for '{name}' at endpoint '{}'",
                                error.endpoint
                            )))
                        }
                        ProviderErrorKind::InvalidConfiguration => {
                            return Err(IntentError::runtime_error(format!(
                                "Secret provider configuration is invalid for endpoint '{}'",
                                error.endpoint
                            )))
                        }
                    },
                }
            }
        }

        Err(IntentError::runtime_error(format!(
            "Secret provider unavailable after {attempts} bounded attempt(s) across {} endpoint(s): {}",
            unavailable.len(),
            unavailable
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn endpoint(&self) -> &ProviderEndpointLabel {
        static ENDPOINT: OnceLock<ProviderEndpointLabel> = OnceLock::new();
        ENDPOINT.get_or_init(ProviderEndpointLabel::env)
    }

    fn authorization_scope(&self) -> &str {
        "process-env"
    }

    fn lookup(&self, name: &str) -> std::result::Result<ProviderLookup, ProviderError> {
        match std::env::var(name) {
            Ok(value) if value.is_empty() => Ok(ProviderLookup::Missing),
            Ok(value) => Ok(ProviderLookup::Found(Zeroizing::new(value))),
            Err(std::env::VarError::NotPresent) => Ok(ProviderLookup::Missing),
            Err(std::env::VarError::NotUnicode(_)) => Err(ProviderError::new(
                ProviderErrorKind::InvalidConfiguration,
                self.endpoint(),
            )),
        }
    }
}

struct SocketProviderConfig {
    endpoints: Vec<PathBuf>,
    authorization_scope: String,
    timeout: Duration,
}

fn socket_config_error() -> IntentError {
    IntentError::runtime_error("Larri socket secrets provider configuration is invalid".to_string())
}

#[cfg(unix)]
fn validate_no_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(socket_config_error());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(socket_config_error()),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_no_symlink_components(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn validate_existing_socket_path(path: &Path) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;

    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() => {
            Err(socket_config_error())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(socket_config_error()),
    }
}

#[cfg(not(unix))]
fn validate_existing_socket_path(_path: &Path) -> Result<()> {
    Ok(())
}

fn parse_socket_provider_config(
    endpoints: Option<&str>,
    authorization_scope: Option<&str>,
    timeout_ms: Option<&str>,
    production: bool,
) -> Result<SocketProviderConfig> {
    let raw_endpoints = endpoints.ok_or_else(socket_config_error)?;
    let parts: Vec<&str> = raw_endpoints.split(',').map(str::trim).collect();
    if parts.len() > MAX_SOCKET_ENDPOINTS {
        return Err(socket_config_error());
    }

    let mut seen = HashSet::with_capacity(parts.len());
    let mut parsed_endpoints = Vec::with_capacity(parts.len());
    for endpoint in parts {
        if endpoint.is_empty()
            || endpoint.len() > MAX_SOCKET_PATH_BYTES
            || endpoint.chars().any(char::is_control)
        {
            return Err(socket_config_error());
        }
        let path = PathBuf::from(endpoint);
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
            || (production
                && (path == Path::new(PRODUCTION_SOCKET_ROOT)
                    || !path.starts_with(PRODUCTION_SOCKET_ROOT)))
            || !seen.insert(path.clone())
        {
            return Err(socket_config_error());
        }
        if production {
            validate_no_symlink_components(&path)?;
        }
        validate_existing_socket_path(&path)?;
        parsed_endpoints.push(path);
    }

    let authorization_scope = authorization_scope.ok_or_else(socket_config_error)?;
    if authorization_scope != authorization_scope.trim()
        || authorization_scope.is_empty()
        || authorization_scope.len() > MAX_AUTHORIZATION_SCOPE_BYTES
        || !authorization_scope.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | ':' | '-')
        })
    {
        return Err(socket_config_error());
    }

    let timeout_ms = match timeout_ms {
        Some(value) => value.parse::<u64>().map_err(|_| socket_config_error())?,
        None => DEFAULT_SOCKET_TIMEOUT_MS,
    };
    if !(MIN_SOCKET_TIMEOUT_MS..=MAX_SOCKET_TIMEOUT_MS).contains(&timeout_ms) {
        return Err(socket_config_error());
    }

    Ok(SocketProviderConfig {
        endpoints: parsed_endpoints,
        authorization_scope: authorization_scope.to_string(),
        timeout: Duration::from_millis(timeout_ms),
    })
}

fn is_production_mode() -> bool {
    std::env::var("NTNT_ENV")
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "production" | "prod"))
        .unwrap_or(false)
}

fn configured_provider_group_from_values(
    provider: &str,
    socket_endpoints: Option<&str>,
    authorization_scope: Option<&str>,
    timeout_ms: Option<&str>,
    production: bool,
) -> Result<ProviderGroup> {
    match provider {
        "env" if production => Err(IntentError::runtime_error(
            "The environment secrets provider is development-only and is disabled in production"
                .to_string(),
        )),
        "env" => ProviderGroup::new(vec![Arc::new(EnvSecretProvider)]),
        "larri-socket" => {
            let config = parse_socket_provider_config(
                socket_endpoints,
                authorization_scope,
                timeout_ms,
                production,
            )?;
            #[cfg(unix)]
            {
                let providers = config
                    .endpoints
                    .into_iter()
                    .enumerate()
                    .map(|(index, path)| {
                        let endpoint = ProviderEndpointLabel::socket(index + 1);
                        let provider = if production {
                            socket::SocketSecretProvider::new_with_trusted_root(
                                path,
                                endpoint,
                                config.authorization_scope.clone(),
                                config.timeout,
                                PathBuf::from(PRODUCTION_SOCKET_ROOT),
                            )
                        } else {
                            socket::SocketSecretProvider::new(
                                path,
                                endpoint,
                                config.authorization_scope.clone(),
                                config.timeout,
                            )
                        };
                        Arc::new(provider) as Arc<dyn SecretProvider>
                    })
                    .collect();
                ProviderGroup::new(providers)
            }
            #[cfg(not(unix))]
            {
                let _ = config;
                Err(IntentError::runtime_error(
                    "The Larri socket secrets provider is supported only on Unix platforms"
                        .to_string(),
                ))
            }
        }
        _ => Err(IntentError::runtime_error(
            "Unsupported secrets provider; supported providers are 'env' and 'larri-socket'"
                .to_string(),
        )),
    }
}

fn configured_provider_group() -> Result<ProviderGroup> {
    let provider = std::env::var(PROVIDER_ENV).unwrap_or_else(|_| "env".to_string());
    let socket_endpoints = std::env::var(SOCKET_ENDPOINTS_ENV).ok();
    let authorization_scope = std::env::var(SOCKET_SCOPE_ENV).ok();
    let timeout_ms = std::env::var(SOCKET_TIMEOUT_ENV).ok();
    configured_provider_group_from_values(
        &provider,
        socket_endpoints.as_deref(),
        authorization_scope.as_deref(),
        timeout_ms.as_deref(),
        is_production_mode(),
    )
}

fn lookup_secret(name: &str) -> Result<Option<SecretValue>> {
    // Validate before the name can reach declaration diagnostics or a provider.
    // SecretValue validates again only to preserve its own construction invariant.
    validate_secret_name(name)?;
    enforce_declared(name)?;
    configured_provider_group()?.lookup(name)
}

fn secret_name_arg(args: &[Value], function: &str) -> Result<String> {
    match args.first() {
        Some(Value::String(name)) => Ok(name.clone()),
        _ => Err(IntentError::type_error(format!(
            "{function}() requires a secret name string"
        ))),
    }
}

/// Initialize the `std/secrets` module.
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt get_secret
    // @module std/secrets
    // @module_description Provider-neutral secret lookup with opaque, redacted values
    // @signature get_secret(name: String) -> Option<Secret>
    // Looks up a secret by its provider-neutral logical name.
    //
    // The environment provider reads the exact environment variable name and is
    // disabled when `NTNT_ENV` is `production` or `prod`. Unix deployments can
    // instead select the Larri host-agent provider with
    // `NTNT_SECRETS_PROVIDER=larri-socket`, a comma-separated list of absolute
    // `NTNT_SECRETS_SOCKET_ENDPOINTS`, and the expected non-credential deployment
    // identifier in `NTNT_SECRETS_AUTHORIZATION_SCOPE`. Optional
    // `NTNT_SECRETS_TIMEOUT_MS` is bounded from 10 through 10000 milliseconds.
    // Production socket paths must be beneath `/run/larri-secrets`; endpoints are
    // retried twice and failed over only for bounded unavailable results. Plaintext
    // caching remains in the host agent rather than ntnt.
    // Projects with an `ntnt.toml` must declare accessible names under
    // `[secrets.<NAME>]`; undeclared lookups fail before contacting the provider.
    // Declaration metadata may contain `label`, `description`, `required`, and
    // `environments`; secret values are never accepted in the manifest.
    // Secret values remain opaque and redact themselves in output and diagnostics.
    // @param name A validated logical secret name
    // @returns Some(Secret) when configured, otherwise None
    // @see_also require_secret
    // @since v0.5.1
    // @tags #security, #secrets
    // @example get_secret("STRIPE_SECRET_KEY") => Some([REDACTED]) ~ "Optional lookup"
    // @error RuntimeError ~ "Unsupported secrets provider" fix: "Select env for development or larri-socket with deployment-scoped socket configuration"
    module.insert(
        "get_secret".to_string(),
        Value::NativeFunction {
            name: "get_secret".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let name = secret_name_arg(args, "get_secret")?;
                Ok(match lookup_secret(&name)? {
                    Some(secret) => Value::some(Value::Secret(secret)),
                    None => Value::none(),
                })
            },
        },
    );

    // @ntnt require_secret
    // @module std/secrets
    // @module_description Provider-neutral secret lookup with opaque, redacted values
    // @signature require_secret(name: String) -> Secret
    // Looks up a required secret and fails closed when it is not configured.
    //
    // Use this at startup or immediately before an approved secret-consuming sink.
    // In v0.5.1, `std/http.fetch` accepts Secret values as header, cookie,
    // basic-auth, raw body, JSON-leaf, and form values. These requests require HTTPS;
    // APP_ENV=development permits direct HTTP only for localhost and loopback IPs.
    // Templates, public JSON, URL/CSV/string conversion, KV storage, and job payloads
    // reject secrets.
    // There is intentionally no general Secret-to-String reveal function.
    // The error identifies only the logical name and never includes the value.
    // @param name A validated logical secret name
    // @returns The opaque Secret value
    // @see_also get_secret
    // @since v0.5.1
    // @tags #security, #secrets
    // @example require_secret("STRIPE_SECRET_KEY") => [REDACTED] ~ "Required lookup"
    // @error RuntimeError ~ "Required secret is not configured" fix: "Configure the named secret in the selected provider"
    module.insert(
        "require_secret".to_string(),
        Value::NativeFunction {
            name: "require_secret".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let name = secret_name_arg(args, "require_secret")?;
                match lookup_secret(&name)? {
                    Some(secret) => Ok(Value::Secret(secret)),
                    None => Err(IntentError::runtime_error(format!(
                        "Required secret '{name}' is not configured"
                    ))),
                }
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn socket_provider_config_is_bounded_and_production_scoped() {
        let config = parse_socket_provider_config(
            Some("/tmp/one.sock,/tmp/two.sock"),
            Some("deployment-a"),
            Some("250"),
            false,
        )
        .expect("valid development socket config");
        assert_eq!(config.endpoints.len(), 2);
        assert_eq!(config.authorization_scope, "deployment-a");
        assert_eq!(config.timeout, std::time::Duration::from_millis(250));
        parse_socket_provider_config(
            Some("/run/larri-secrets/agent.sock"),
            Some("deployment-a"),
            None,
            true,
        )
        .expect("valid production socket root");

        for (endpoints, scope, timeout, production) in [
            (None, Some("deployment-a"), None, false),
            (Some("/tmp/a.sock"), None, None, false),
            (Some("/tmp/a.sock"), Some(" deployment-a "), None, false),
            (Some("relative.sock"), Some("deployment-a"), None, false),
            (Some("/tmp/bad\n.sock"), Some("deployment-a"), None, false),
            (Some("/run/larri-secrets"), Some("deployment-a"), None, true),
            (
                Some("/tmp/a.sock,/tmp/a.sock"),
                Some("deployment-a"),
                None,
                false,
            ),
            (
                Some("/run/larri-secrets/../other.sock"),
                Some("deployment-a"),
                None,
                true,
            ),
            (Some("/tmp/a.sock"), Some("deployment-a"), None, true),
            (Some("/tmp/a.sock"), Some("deployment-a"), Some("9"), false),
            (
                Some("/tmp/a.sock"),
                Some("deployment-a"),
                Some("10001"),
                false,
            ),
            (Some("/tmp/a.sock"), Some("deployment\nscope"), None, false),
        ] {
            assert!(
                parse_socket_provider_config(endpoints, scope, timeout, production).is_err(),
                "invalid socket configuration must fail closed"
            );
        }

        let too_many = (0..9)
            .map(|index| format!("/tmp/{index}.sock"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            parse_socket_provider_config(Some(&too_many), Some("deployment-a"), None, false,)
                .is_err()
        );

        let overlong = format!("/tmp/{}.sock", "a".repeat(96));
        assert!(
            parse_socket_provider_config(Some(&overlong), Some("deployment-a"), None, false,)
                .is_err()
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn socket_provider_selector_fails_closed_off_unix() {
        let result = configured_provider_group_from_values(
            "larri-socket",
            Some(r"C:\run\larri-secrets\agent.sock"),
            Some("deployment-a"),
            Some("250"),
            false,
        );
        let error = match result {
            Ok(_) => panic!("socket provider must be unavailable off Unix"),
            Err(error) => error,
        };
        let rendered = error.to_string();
        assert!(rendered.contains("supported only on Unix platforms"));
        assert!(!rendered.contains("agent.sock"));
    }

    #[cfg(unix)]
    #[test]
    fn socket_provider_rejects_symlinked_parent_components() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ntnt-socket-parent-{}-{suffix}",
            std::process::id()
        ));
        let real = root.join("real");
        std::fs::create_dir_all(&real).expect("create real parent");
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("create parent symlink");

        assert!(validate_no_symlink_components(&link.join("agent.sock")).is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn declaration_identity_is_stable_when_manifest_appears() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ntnt-secret-identity-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create project");
        let source = root.join("main.tnt");
        std::fs::write(&source, "print(\"ok\")\n").expect("write source");

        let (without_manifest, _) = load_declarations(&source);
        std::fs::write(
            root.join("ntnt.toml"),
            "[secrets.API_KEY]\nrequired = true\n",
        )
        .expect("write manifest");
        let (with_manifest, declarations) = load_declarations(&source);

        assert_eq!(without_manifest, with_manifest);
        assert!(matches!(declarations, SecretDeclarations::Loaded(_)));
        std::fs::remove_dir_all(root).ok();
    }

    struct MockProvider {
        endpoint: ProviderEndpointLabel,
        authorization_scope: String,
        calls: AtomicUsize,
        results: Mutex<VecDeque<std::result::Result<ProviderLookup, ProviderError>>>,
    }

    impl MockProvider {
        fn new(
            endpoint: &str,
            results: Vec<std::result::Result<ProviderLookup, ProviderError>>,
        ) -> Self {
            Self {
                endpoint: ProviderEndpointLabel::fixture(endpoint),
                authorization_scope: "deployment-a".to_string(),
                calls: AtomicUsize::new(0),
                results: Mutex::new(results.into()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn with_scope(mut self, scope: &str) -> Self {
            self.authorization_scope = scope.to_string();
            self
        }
    }

    impl SecretProvider for MockProvider {
        fn endpoint(&self) -> &ProviderEndpointLabel {
            &self.endpoint
        }

        fn authorization_scope(&self) -> &str {
            &self.authorization_scope
        }

        fn lookup(&self, _name: &str) -> std::result::Result<ProviderLookup, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.results
                .lock()
                .expect("mock results lock")
                .pop_front()
                .expect("mock result")
        }
    }

    fn found(value: &str) -> ProviderLookup {
        ProviderLookup::Found(Zeroizing::new(value.to_string()))
    }

    fn provider_error(kind: ProviderErrorKind, endpoint: &str) -> ProviderError {
        ProviderError::new(kind, &ProviderEndpointLabel::fixture(endpoint))
    }

    #[test]
    fn provider_group_rejects_mixed_authorization_scopes() {
        let first = Arc::new(
            MockProvider::new("socket-a", vec![Ok(ProviderLookup::Missing)])
                .with_scope("deployment-a"),
        );
        let second = Arc::new(
            MockProvider::new("socket-b", vec![Ok(ProviderLookup::Missing)])
                .with_scope("deployment-b"),
        );

        let result = ProviderGroup::new(vec![first, second]);
        let Err(error) = result else {
            panic!("mixed authorization scopes must fail closed");
        };
        assert!(error.to_string().contains("authorization scope"));
    }

    #[test]
    fn provider_group_fails_over_only_after_unavailable() {
        let first = Arc::new(MockProvider::new(
            "socket-a",
            vec![
                Err(provider_error(ProviderErrorKind::Unavailable, "socket-a")),
                Err(provider_error(ProviderErrorKind::Unavailable, "socket-a")),
            ],
        ));
        let second = Arc::new(MockProvider::new(
            "socket-b",
            vec![Ok(found("secret-value"))],
        ));
        let group = ProviderGroup::new(vec![first.clone(), second.clone()]).expect("group");

        let value = group.lookup("API_KEY").expect("lookup").expect("found");
        assert_eq!(value.expose(), "secret-value");
        assert_eq!(first.calls(), 2);
        assert_eq!(second.calls(), 1);
    }

    #[test]
    fn provider_group_retries_transient_failure_before_failover() {
        let first = Arc::new(MockProvider::new(
            "socket-a",
            vec![
                Err(provider_error(ProviderErrorKind::Unavailable, "socket-a")),
                Ok(found("secret-value")),
            ],
        ));
        let second = Arc::new(MockProvider::new(
            "socket-b",
            vec![Ok(found("must-not-read"))],
        ));
        let group = ProviderGroup::new(vec![first.clone(), second.clone()]).expect("group");

        let value = group.lookup("API_KEY").expect("lookup").expect("found");
        assert_eq!(value.expose(), "secret-value");
        assert_eq!(first.calls(), 2);
        assert_eq!(second.calls(), 0);
    }

    #[test]
    fn provider_group_stops_on_access_denied() {
        let first = Arc::new(MockProvider::new(
            "socket-a",
            vec![Err(provider_error(
                ProviderErrorKind::AccessDenied,
                "socket-a",
            ))],
        ));
        let second = Arc::new(MockProvider::new(
            "socket-b",
            vec![Ok(found("must-not-read"))],
        ));
        let group = ProviderGroup::new(vec![first, second.clone()]).expect("group");

        let err = group.lookup("API_KEY").expect_err("access denied");
        assert!(err.to_string().contains("access denied"));
        assert_eq!(second.calls(), 0);
    }

    #[test]
    fn provider_group_stops_on_missing() {
        let first = Arc::new(MockProvider::new(
            "socket-a",
            vec![Ok(ProviderLookup::Missing)],
        ));
        let second = Arc::new(MockProvider::new(
            "socket-b",
            vec![Ok(found("must-not-read"))],
        ));
        let group = ProviderGroup::new(vec![first, second.clone()]).expect("group");

        assert!(group.lookup("API_KEY").expect("lookup").is_none());
        assert_eq!(second.calls(), 0);
    }

    #[test]
    fn provider_group_reports_bounded_all_unavailable_attempts() {
        let providers: Vec<Arc<dyn SecretProvider>> = ["socket-a", "socket-b"]
            .into_iter()
            .map(|endpoint| {
                Arc::new(MockProvider::new(
                    endpoint,
                    vec![
                        Err(provider_error(ProviderErrorKind::Unavailable, endpoint)),
                        Err(provider_error(ProviderErrorKind::Unavailable, endpoint)),
                    ],
                )) as Arc<dyn SecretProvider>
            })
            .collect();
        let group = ProviderGroup::new(providers).expect("group");

        let err = group.lookup("API_KEY").expect_err("all unavailable");
        let rendered = err.to_string();
        assert!(rendered.contains("4 bounded attempt(s) across 2 endpoint(s)"));
        assert!(rendered.contains("socket-a, socket-b"));
        assert!(!rendered.contains("secret-value"));
    }
}
