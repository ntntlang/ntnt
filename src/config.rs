//! Configuration for NTNT runtime and lint modes.
//!
//! Provides [`TypeMode`] (runtime behavior for type mismatches) and [`LintMode`]
//! (lint-time behavior for missing annotations), read from environment variables.
//! Values are cached via `OnceLock` in non-test builds (`#[cfg(not(test))]`);
//! re-read on every call in test builds so that tests can manipulate env vars
//! with isolation.

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
    // In tests, use thread-local override only (no env var reads).
    // std::env::var is unsafe to call concurrently with set_var on macOS
    // (Rust 1.83+ / POSIX getenv is not thread-safe on all platforms).
    TYPE_MODE_OVERRIDE.with(|cell| (*cell.borrow()).unwrap_or(TypeMode::Warn))
}

thread_local! {
    /// Thread-local override for TypeMode in tests. Avoids `std::env::set_var`
    /// which is unsafe in multi-threaded contexts (Rust 1.83+).
    static TYPE_MODE_OVERRIDE: RefCell<Option<TypeMode>> = const { RefCell::new(None) };
}

/// Set a thread-local TypeMode override for the current test. Returns a guard
/// that restores `None` on drop. Use instead of `std::env::set_var("NTNT_TYPE_MODE", ...)`.
#[cfg(test)]
pub fn set_test_type_mode(mode: TypeMode) -> TestTypeModeGuard {
    TYPE_MODE_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(mode));
    TestTypeModeGuard
}

/// RAII guard that clears the thread-local TypeMode override on drop.
#[cfg(test)]
pub struct TestTypeModeGuard;

#[cfg(test)]
impl Drop for TestTypeModeGuard {
    fn drop(&mut self) {
        TYPE_MODE_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
    }
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
    // In tests, always return Default (no env var reads for thread safety).
    LintMode::Default
}

use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    /// Tracks already-warned type mismatch locations to prevent duplicate warnings
    /// in the same evaluation context (e.g., a template for-loop iterating 50 bad rows).
    static WARNED_LOCATIONS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Log a type-mode warning to stderr, deduplicating by message key.
/// Returns `true` if the warning was emitted, `false` if it was suppressed
/// as a duplicate.
pub fn type_warn_dedup(key: &str, message: &str) -> bool {
    WARNED_LOCATIONS.with(|warned| {
        let mut set = warned.borrow_mut();
        if set.contains(key) {
            false
        } else {
            set.insert(key.to_string());
            eprintln!("[WARN] {}", message);
            true
        }
    })
}

/// Clear the warning dedup set. Call at the start of each request
/// or evaluation to allow warnings to fire again for the next request.
pub fn clear_type_warnings() {
    WARNED_LOCATIONS.with(|warned| warned.borrow_mut().clear());
}
