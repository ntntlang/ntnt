//! Configuration for NTNT runtime and lint modes.
//!
//! Provides [`TypeMode`] (runtime behavior for type mismatches) and [`LintMode`]
//! (lint-time behavior for missing annotations), read from environment variables.
//! Values are cached via `OnceLock` in production builds; re-read on every call
//! in test builds so that tests can manipulate env vars with isolation.

/// Runtime type safety mode, controlled by the `NTNT_TYPE_MODE` env var.
///
/// Controls how runtime type mismatches are handled in:
/// - Index operations (`[]`) on unexpected types
/// - `for..in` on non-collection values
/// - Template expression errors
///
/// | Value | Behaviour |
/// |-------|-----------|
/// | `strict` | Type mismatches are runtime errors (program halts) |
/// | `warn` | Type mismatches emit a warning to stderr and continue **(default)** |
/// | `forgiving` | Type mismatches are silently ignored |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeMode {
    /// Type mismatches are runtime errors — the program halts.
    Strict,
    /// Type mismatches emit `[WARN]` to stderr and continue (default).
    Warn,
    /// Type mismatches are silently ignored (pre-v0.4 behaviour).
    Forgiving,
}

/// Lint-time type annotation mode, controlled by `NTNT_LINT_MODE` or CLI flags.
///
/// Controls how the linter treats functions without type annotations.
///
/// | Value | Behaviour |
/// |-------|-----------|
/// | `default` | Only report type errors where annotations exist **(default)** |
/// | `warn` | Also emit warnings for functions missing type annotations |
/// | `strict` | Missing annotations are lint errors (non-zero exit code) |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintMode {
    /// Only report type errors where annotations exist (default behaviour).
    Default,
    /// Also emit warnings for functions missing type annotations.
    Warn,
    /// Missing annotations are lint errors (non-zero exit code).
    Strict,
}

fn read_type_mode_from_env() -> TypeMode {
    match std::env::var("NTNT_TYPE_MODE").as_deref().unwrap_or("warn") {
        "strict" => TypeMode::Strict,
        "forgiving" => TypeMode::Forgiving,
        _ => TypeMode::Warn,
    }
}

/// Get the current runtime type mode from the `NTNT_TYPE_MODE` env var.
///
/// Default is [`TypeMode::Warn`]. In production builds the result is cached on
/// first call; in test builds it reads fresh from the environment every call
/// so tests can manipulate `NTNT_TYPE_MODE` with per-test isolation.
#[cfg(not(test))]
pub fn get_type_mode() -> TypeMode {
    use std::sync::OnceLock;
    static TYPE_MODE: OnceLock<TypeMode> = OnceLock::new();
    *TYPE_MODE.get_or_init(read_type_mode_from_env)
}

#[cfg(test)]
pub fn get_type_mode() -> TypeMode {
    read_type_mode_from_env()
}

fn read_lint_mode_from_env() -> LintMode {
    match std::env::var("NTNT_LINT_MODE")
        .as_deref()
        .unwrap_or("default")
    {
        "warn" => LintMode::Warn,
        "strict" => LintMode::Strict,
        _ => LintMode::Default,
    }
}

/// Get the current lint mode from the `NTNT_LINT_MODE` env var.
///
/// Default is [`LintMode::Default`]. CLI flags take precedence over this value
/// (caller is responsible for applying the override). In production builds the
/// result is cached; in test builds it reads fresh each call.
#[cfg(not(test))]
pub fn get_lint_mode() -> LintMode {
    use std::sync::OnceLock;
    static LINT_MODE: OnceLock<LintMode> = OnceLock::new();
    *LINT_MODE.get_or_init(read_lint_mode_from_env)
}

#[cfg(test)]
pub fn get_lint_mode() -> LintMode {
    read_lint_mode_from_env()
}
