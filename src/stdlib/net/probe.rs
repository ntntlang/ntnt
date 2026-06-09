use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeProtocol {
    Icmp,
    Tcp,
}

impl ProbeProtocol {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Icmp => "icmp",
            Self::Tcp => "tcp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeErrorKind {
    ResolveFailed,
    PolicyDenied,
    CapabilityUnavailable,
    PermissionDenied,
    #[cfg(not(target_os = "linux"))]
    UnsupportedPlatform,
    SystemFailure,
    UnexpectedResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProbeError {
    kind: ProbeErrorKind,
    protocol: Option<ProbeProtocol>,
    target: Option<String>,
    message: String,
}

impl ProbeError {
    pub(super) fn new(
        kind: ProbeErrorKind,
        protocol: Option<ProbeProtocol>,
        target: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            protocol,
            target,
            message: message.into(),
        }
    }

    pub(super) fn resolve_failed(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            ProbeErrorKind::ResolveFailed,
            None,
            Some(target.into()),
            message,
        )
    }

    pub(super) fn policy_denied(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            ProbeErrorKind::PolicyDenied,
            None,
            Some(target.into()),
            message,
        )
    }

    pub(super) fn capability_unavailable(
        protocol: ProbeProtocol,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            ProbeErrorKind::CapabilityUnavailable,
            Some(protocol),
            None,
            message,
        )
    }

    pub(super) fn permission_denied(protocol: ProbeProtocol, message: impl Into<String>) -> Self {
        Self::new(
            ProbeErrorKind::PermissionDenied,
            Some(protocol),
            None,
            message,
        )
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn unsupported_platform(
        protocol: ProbeProtocol,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            ProbeErrorKind::UnsupportedPlatform,
            Some(protocol),
            None,
            message,
        )
    }

    pub(super) fn system_failure(protocol: ProbeProtocol, message: impl Into<String>) -> Self {
        Self::new(ProbeErrorKind::SystemFailure, Some(protocol), None, message)
    }

    pub(super) fn unexpected_result(protocol: ProbeProtocol, message: impl Into<String>) -> Self {
        Self::new(
            ProbeErrorKind::UnexpectedResult,
            Some(protocol),
            None,
            message,
        )
    }

    #[cfg(test)]
    pub(super) fn kind(&self) -> ProbeErrorKind {
        self.kind
    }

    #[cfg(test)]
    pub(super) fn protocol(&self) -> Option<ProbeProtocol> {
        self.protocol
    }
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProbeOptions {
    pub(super) timeout: Duration,
    pub(super) count: usize,
    pub(super) interval: Duration,
    pub(super) allow_private: bool,
}
