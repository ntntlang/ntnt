//! Opaque runtime storage for secret values.

use crate::error::{IntentError, Result};
use std::fmt;
use std::sync::Arc;
use zeroize::Zeroizing;

/// Redacted marker used by every ordinary formatting path.
pub const REDACTED_SECRET: &str = "[REDACTED]";

/// A secret value with a validated, non-sensitive logical name.
///
/// Clones share one zeroizing allocation. Approved outbound sinks may borrow the
/// plaintext, but ordinary formatting and debugging never expose it.
#[derive(Clone)]
pub struct SecretValue {
    name: Arc<str>,
    value: Arc<Zeroizing<String>>,
}

impl SecretValue {
    /// Construct a secret after validating its logical name.
    pub(crate) fn new(name: impl Into<String>, value: impl Into<String>) -> Result<Self> {
        let name = name.into();
        validate_secret_name(&name)?;
        Ok(Self {
            name: Arc::from(name),
            value: Arc::new(Zeroizing::new(value.into())),
        })
    }

    /// The validated, non-sensitive logical name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow plaintext for an audited sink. Keep visibility crate-local.
    pub(crate) fn expose(&self) -> &str {
        self.value.as_str()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretValue")
            .field("name", &self.name)
            .field("value", &REDACTED_SECRET)
            .finish()
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_SECRET)
    }
}

/// Validate provider-neutral secret identifiers before they reach diagnostics or providers.
pub fn validate_secret_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(IntentError::type_error(
            "Secret names must contain between 1 and 128 characters".to_string(),
        ));
    }

    let mut chars = name.chars();
    let first = chars.next().expect("non-empty checked above");
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(IntentError::type_error(
            "Secret names must match [A-Za-z_][A-Za-z0-9_.-]*".to_string(),
        ));
    }

    Ok(())
}
