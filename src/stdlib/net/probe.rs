//! Shared probe substrate for `std/net` diagnostics.
//!
//! Probe drivers (ICMP ping today, traceroute and richer TCP probing later)
//! share two concerns that must behave identically everywhere:
//!
//! 1. **Failure classification.** A probe can fail because the *target* is
//!    unreachable (a valid diagnostic outcome, reported per-attempt) or
//!    because the *backend* could not probe at all (permissions, socket
//!    setup, impossible configuration — surfaced as `Err` to the caller).
//!    [`ProbeFailure`] carries that distinction as a type so it survives
//!    every layer; string sniffing on error messages is not allowed.
//!
//! 2. **Deadline budgeting.** A probe sequence shares one global deadline
//!    across attempts and inter-attempt intervals. [`probe_attempt_budget`]
//!    divides the remaining budget so a requested count either completes or
//!    fails loudly — it never silently shrinks.

use std::time::Duration;

/// Why a probe failed, preserved as a type across all probe layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeFailure {
    /// The target itself is unreachable: no route, host down, or an ICMP
    /// error quoting our probe. This is a valid probe outcome — drivers
    /// record it as a failed attempt rather than aborting.
    Target(String),
    /// The probe machinery failed: permissions, socket errors, or a
    /// configuration that cannot be satisfied. Says nothing about the
    /// target and propagates as `Err` to the caller.
    Backend(String),
}

impl ProbeFailure {
    pub(crate) fn is_target(&self) -> bool {
        matches!(self, ProbeFailure::Target(_))
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            ProbeFailure::Target(message) | ProbeFailure::Backend(message) => message,
        }
    }
}

impl std::fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeFailure::Target(message) | ProbeFailure::Backend(message) => f.write_str(message),
        }
    }
}

/// Splits the remaining global deadline across the outstanding attempts of a
/// probe sequence, reserving room for the intervals between them.
///
/// Returns the time budget for the next attempt, or a backend failure when
/// the remaining budget cannot fit the outstanding attempts plus intervals.
/// `label` names the calling probe (e.g. "ICMP ping") in error messages.
pub(crate) fn probe_attempt_budget(
    label: &str,
    remaining: Duration,
    remaining_attempts: usize,
    interval: Duration,
) -> Result<Duration, ProbeFailure> {
    let attempts = remaining_attempts.max(1);
    let future_intervals = interval
        .checked_mul(attempts.saturating_sub(1) as u32)
        .ok_or_else(|| ProbeFailure::Backend(format!("{label} interval budget overflowed")))?;
    if remaining <= future_intervals {
        return Err(ProbeFailure::Backend(format!(
            "{label} timeout_ms is too small for count {} and interval_ms {}",
            remaining_attempts,
            interval.as_millis()
        )));
    }
    let attempt_budget = (remaining - future_intervals) / (attempts as u32);
    if attempt_budget.is_zero() {
        return Err(ProbeFailure::Backend(format!(
            "{label} timeout_ms is too small to send requested probes"
        )));
    }
    Ok(attempt_budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_attempt_budget_rejects_impossible_count_interval() {
        let failure = probe_attempt_budget(
            "ICMP ping",
            Duration::from_millis(100),
            3,
            Duration::from_millis(60),
        )
        .unwrap_err();
        assert!(!failure.is_target());
        assert!(failure.into_message().contains("timeout_ms is too small"));
    }

    #[test]
    fn probe_attempt_budget_divides_remaining_across_attempts() {
        let budget = probe_attempt_budget(
            "ICMP ping",
            Duration::from_millis(900),
            3,
            Duration::from_millis(100),
        )
        .unwrap();
        // 900ms minus two 100ms intervals leaves 700ms across three probes.
        assert_eq!(budget, Duration::from_millis(700) / 3);
    }

    #[test]
    fn probe_failure_classification_is_typed() {
        let target = ProbeFailure::Target("host down".to_string());
        let backend = ProbeFailure::Backend("permission denied".to_string());
        assert!(target.is_target());
        assert!(!backend.is_target());
        assert_eq!(target.into_message(), "host down");
        assert_eq!(backend.to_string(), "permission denied");
    }
}
