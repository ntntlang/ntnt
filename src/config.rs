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

#[cfg_attr(test, allow(dead_code))]
/// Default type mode when NTNT_TYPE_MODE is unset. Verification commands
/// (`ntnt intent check`, `ntnt test`) set this to Strict before the first
/// get_type_mode() read so verification means verification (DD-063 Rec 7);
/// an explicit NTNT_TYPE_MODE always wins.
static TYPE_MODE_DEFAULT: std::sync::OnceLock<TypeMode> = std::sync::OnceLock::new();

/// Set the default type mode used when NTNT_TYPE_MODE is unset.
/// Must be called before the first `get_type_mode()` read — a late call
/// cannot take effect (the resolved mode is already cached), so it is
/// surfaced loudly instead of silently leaving verification in warn mode.
pub fn set_default_type_mode(mode: TypeMode) {
    #[cfg(not(test))]
    if TYPE_MODE_CACHE.get().is_some() {
        eprintln!(
            "[WARN] set_default_type_mode({:?}) called after the type mode was already resolved — default not applied",
            mode
        );
        debug_assert!(
            false,
            "set_default_type_mode must run before the first get_type_mode() read"
        );
        return;
    }
    let _ = TYPE_MODE_DEFAULT.set(mode);
}

#[cfg(not(test))]
fn read_type_mode_from_env() -> TypeMode {
    match std::env::var("NTNT_TYPE_MODE").as_deref() {
        Ok("strict") => TypeMode::Strict,
        Ok("forgiving") => TypeMode::Forgiving,
        Ok(_) => TypeMode::Warn,
        Err(_) => TYPE_MODE_DEFAULT.get().copied().unwrap_or(TypeMode::Warn),
    }
}

/// Get the current runtime type mode.
///
/// Default is [`TypeMode::Warn`]. In production builds the result is read from
/// `NTNT_TYPE_MODE` env var and cached via `OnceLock`. In test builds, a
/// thread-local override is used instead of env vars (since `std::env::set_var`
/// is unsafe in multi-threaded contexts on Rust 1.83+). Use
/// [`set_test_type_mode`] to override in tests.
#[cfg(not(test))]
static TYPE_MODE_CACHE: std::sync::OnceLock<TypeMode> = std::sync::OnceLock::new();

#[cfg(not(test))]
pub fn get_type_mode() -> TypeMode {
    *TYPE_MODE_CACHE.get_or_init(read_type_mode_from_env)
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

#[cfg_attr(test, allow(dead_code))]
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

/// Get the current lint mode.
///
/// Default is [`LintMode::Default`]. In production builds the result is read
/// from `NTNT_LINT_MODE` env var and cached via `OnceLock`. CLI flags take
/// precedence (caller applies the override). In test builds, always returns
/// `LintMode::Default` for thread safety (no env var reads). Add a thread-local
/// override similar to `TypeMode` if lint-mode testing is needed.
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
