//! Provider-neutral secret lookup for ntnt applications.

use crate::error::{IntentError, Result};
use crate::interpreter::{SecretValue, Value};
use crate::secret::validate_secret_name;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

const PROVIDER_ENV: &str = "NTNT_SECRETS_PROVIDER";
const DEFAULT_ATTEMPTS_PER_ENDPOINT: usize = 2;

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
    if let Ok(mut state) = declaration_state().write() {
        match &state.identity {
            None => {
                state.identity = Some(identity);
                state.declarations = declarations;
            }
            Some(current) if current == &identity => state.declarations = declarations,
            Some(_) => state.declarations = SecretDeclarations::ConflictingApplication,
        }
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
    let identity = manifest.canonicalize().unwrap_or_else(|_| manifest.clone());

    let Ok(content) = std::fs::read_to_string(manifest) else {
        return (identity, SecretDeclarations::Invalid);
    };
    let Ok(document) = content.parse::<toml::Value>() else {
        return (identity, SecretDeclarations::Invalid);
    };
    let Some(secrets) = document.get("secrets") else {
        return (identity, SecretDeclarations::Loaded(HashSet::new()));
    };
    let Some(table) = secrets.as_table() else {
        return (identity, SecretDeclarations::Invalid);
    };

    let mut declared = HashSet::with_capacity(table.len());
    for (name, metadata) in table {
        let Some(metadata) = metadata.as_table() else {
            return (identity, SecretDeclarations::Invalid);
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
            return (identity, SecretDeclarations::Invalid);
        }
        declared.insert(name.clone());
    }
    (identity, SecretDeclarations::Loaded(declared))
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
    Found(String),
    Missing,
}

/// Stable failure classes for HA failover. The v0.5.1 environment provider
/// can only emit invalid configuration; socket providers use the remaining classes.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderErrorKind {
    Unavailable,
    AccessDenied,
    InvalidConfiguration,
}

#[derive(Debug, Clone)]
struct ProviderError {
    kind: ProviderErrorKind,
    endpoint: String,
}

impl ProviderError {
    fn new(kind: ProviderErrorKind, endpoint: impl Into<String>) -> Self {
        Self {
            kind,
            endpoint: sanitize_endpoint(&endpoint.into()),
        }
    }
}

trait SecretProvider: Send + Sync {
    fn endpoint(&self) -> &str;
    fn authorization_scope(&self) -> &str;

    /// Perform one endpoint lookup. Implementations must apply a bounded timeout
    /// and classify timeouts/transient transport failures as `Unavailable`.
    fn lookup(&self, name: &str) -> std::result::Result<ProviderLookup, ProviderError>;
}

/// Ordered equivalent provider endpoints with classified failover.
///
/// Only `Unavailable` advances to the next endpoint. Missing, denied, and invalid
/// configuration results are terminal so failover cannot change authorization scope.
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
        validate_secret_name(name)?;
        let mut unavailable = Vec::new();
        let mut attempts = 0;

        for provider in &self.providers {
            for attempt in 0..self.attempts_per_endpoint {
                attempts += 1;
                match provider.lookup(name) {
                    Ok(ProviderLookup::Found(value)) => {
                        return SecretValue::new(name, value).map(Some)
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
            unavailable.join(", ")
        )))
    }
}

struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn endpoint(&self) -> &str {
        "env"
    }

    fn authorization_scope(&self) -> &str {
        "process-env"
    }

    fn lookup(&self, name: &str) -> std::result::Result<ProviderLookup, ProviderError> {
        match std::env::var(name) {
            Ok(value) if value.is_empty() => Ok(ProviderLookup::Missing),
            Ok(value) => Ok(ProviderLookup::Found(value)),
            Err(std::env::VarError::NotPresent) => Ok(ProviderLookup::Missing),
            Err(std::env::VarError::NotUnicode(_)) => Err(ProviderError::new(
                ProviderErrorKind::InvalidConfiguration,
                self.endpoint(),
            )),
        }
    }
}

fn sanitize_endpoint(endpoint: &str) -> String {
    if endpoint.is_empty()
        || endpoint.len() > 64
        || !endpoint
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '-'))
    {
        return "unknown".to_string();
    }
    endpoint.to_string()
}

fn is_production_mode() -> bool {
    std::env::var("NTNT_ENV")
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "production" | "prod"))
        .unwrap_or(false)
}

fn configured_provider_group() -> Result<ProviderGroup> {
    let provider = std::env::var(PROVIDER_ENV).unwrap_or_else(|_| "env".to_string());
    match provider.as_str() {
        "env" if is_production_mode() => Err(IntentError::runtime_error(
            "The environment secrets provider is development-only and is disabled in production"
                .to_string(),
        )),
        "env" => ProviderGroup::new(vec![Arc::new(EnvSecretProvider)]),
        _ => Err(IntentError::runtime_error(
            "Unsupported secrets provider; ntnt v0.5.1 supports 'env' in development".to_string(),
        )),
    }
}

fn lookup_secret(name: &str) -> Result<Option<SecretValue>> {
    validate_secret_name(name)?;
    enforce_declared(name)?;
    configured_provider_group()?.lookup(name)
}

fn secret_name_arg(args: &[Value], function: &str) -> Result<String> {
    match args.first() {
        Some(Value::String(name)) => {
            validate_secret_name(name)?;
            Ok(name.clone())
        }
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
    // The v0.5.1 development-only environment provider reads the exact environment
    // variable name and is disabled when `NTNT_ENV` is `production` or `prod`.
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
    // @error RuntimeError ~ "Unsupported secrets provider" fix: "Set NTNT_SECRETS_PROVIDER=env for v0.5.1"
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
    // basic-auth, raw body, JSON-leaf, and form values. Templates, public JSON,
    // URL/CSV/string conversion, KV storage, and job payloads reject secrets.
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

    struct MockProvider {
        endpoint: String,
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
                endpoint: endpoint.to_string(),
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
        fn endpoint(&self) -> &str {
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
                Err(ProviderError::new(
                    ProviderErrorKind::Unavailable,
                    "socket-a",
                )),
                Err(ProviderError::new(
                    ProviderErrorKind::Unavailable,
                    "socket-a",
                )),
            ],
        ));
        let second = Arc::new(MockProvider::new(
            "socket-b",
            vec![Ok(ProviderLookup::Found("secret-value".to_string()))],
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
                Err(ProviderError::new(
                    ProviderErrorKind::Unavailable,
                    "socket-a",
                )),
                Ok(ProviderLookup::Found("secret-value".to_string())),
            ],
        ));
        let second = Arc::new(MockProvider::new(
            "socket-b",
            vec![Ok(ProviderLookup::Found("must-not-read".to_string()))],
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
            vec![Err(ProviderError::new(
                ProviderErrorKind::AccessDenied,
                "socket-a",
            ))],
        ));
        let second = Arc::new(MockProvider::new(
            "socket-b",
            vec![Ok(ProviderLookup::Found("must-not-read".to_string()))],
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
            vec![Ok(ProviderLookup::Found("must-not-read".to_string()))],
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
                        Err(ProviderError::new(ProviderErrorKind::Unavailable, endpoint)),
                        Err(ProviderError::new(ProviderErrorKind::Unavailable, endpoint)),
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
