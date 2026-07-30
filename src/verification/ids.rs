use serde::Serialize;
use std::fmt;

/// Source range for a declaration or diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SourceSpan {
    pub path: String,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl SourceSpan {
    pub fn single_line(
        path: impl Into<String>,
        line: usize,
        start_column: usize,
        end_column: usize,
    ) -> Self {
        Self {
            path: path.into(),
            start_line: line,
            start_column,
            end_line: line,
            end_column,
        }
    }

    pub fn location(&self) -> String {
        format!("{}:{}:{}", self.path, self.start_line, self.start_column)
    }
}

impl fmt::Display for SourceSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.location())
    }
}

/// Whether missing IDs are rejected or derived for legacy Intent files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdMode {
    Strict,
    Compatibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum IdKind {
    Feature,
    Scenario,
    Outcome,
}

impl IdKind {
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Scenario => "scenario",
            Self::Outcome => "outcome",
        }
    }
}

impl fmt::Display for IdKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.prefix())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum IdOrigin {
    Explicit,
    CompatibilityDerived,
}

/// A validated verification identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct StableId {
    value: String,
    kind: IdKind,
    origin: IdOrigin,
}

impl StableId {
    pub fn explicit(value: impl Into<String>, kind: IdKind) -> Result<Self, String> {
        let value = value.into();
        validate_id(&value, kind)?;
        Ok(Self {
            value,
            kind,
            origin: IdOrigin::Explicit,
        })
    }

    pub(crate) fn compatibility_derived(kind: IdKind, labels: &[&str], ordinal: usize) -> Self {
        let label = labels
            .iter()
            .map(|label| slug(label))
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>()
            .join(".");
        let label = if label.is_empty() {
            "unnamed".to_string()
        } else {
            label
        };
        Self {
            value: format!("{}.compat.{}.{}", kind.prefix(), label, ordinal + 1),
            kind,
            origin: IdOrigin::CompatibilityDerived,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub const fn kind(&self) -> IdKind {
        self.kind
    }

    pub const fn origin(&self) -> IdOrigin {
        self.origin
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl AsRef<str> for StableId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A compatibility diagnostic. Derived identity is deliberately visible because
/// it changes when legacy declarations are renamed or reordered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdWarning {
    pub message: String,
    pub id: StableId,
    pub span: SourceSpan,
}

pub(crate) fn validate_id(value: &str, kind: IdKind) -> Result<(), String> {
    let expected_prefix = format!("{}.", kind.prefix());
    let Some(suffix) = value.strip_prefix(&expected_prefix) else {
        return Err(format!("must start with '{expected_prefix}'"));
    };
    if suffix.is_empty() {
        return Err("must contain at least one name segment".to_string());
    }

    for segment in suffix.split('.') {
        if segment.is_empty() {
            return Err("must not contain empty name segments".to_string());
        }
        let bytes = segment.as_bytes();
        if !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
            || !bytes.iter().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'_'
            })
        {
            return Err(
                "segments must use lowercase ASCII letters, digits, '-' or '_', and begin/end with a letter or digit"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn slug(value: &str) -> String {
    const MAX_DERIVED_SEGMENT_BYTES: usize = 48;
    let mut result = String::new();
    let mut separated = false;
    for byte in value.bytes() {
        if result.len() >= MAX_DERIVED_SEGMENT_BYTES {
            break;
        }
        if byte.is_ascii_alphanumeric() {
            result.push((byte as char).to_ascii_lowercase());
            separated = false;
        } else if !separated && !result.is_empty() {
            result.push('-');
            separated = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_kind_prefix_and_segments() {
        assert!(StableId::explicit("outcome.auth.denied-1", IdKind::Outcome).is_ok());
        assert!(StableId::explicit("feature.auth_state", IdKind::Feature).is_ok());
        assert!(StableId::explicit("scenario.auth", IdKind::Outcome).is_err());
        assert!(StableId::explicit("outcome.Auth", IdKind::Outcome).is_err());
        assert!(StableId::explicit("outcome.auth..denied", IdKind::Outcome).is_err());
    }
}
