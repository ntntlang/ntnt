//! Configuration for NTNT runtime and lint modes.
//!
//! Provides [`TypeMode`] (runtime behavior for type mismatches) and [`LintMode`]
//! (lint-time behavior for missing annotations), read from environment variables.
//! Values are cached via `OnceLock` in non-test builds (`#[cfg(not(test))]`);
//! re-read on every call in test builds so that tests can manipulate env vars
//! with isolation.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// One parsed `ntnt.toml` selected by closest-ancestor lookup.
#[derive(Debug, Clone)]
pub struct ProjectManifestDocument {
    path: PathBuf,
    root: PathBuf,
    document: toml::Value,
}

impl ProjectManifestDocument {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn document(&self) -> &toml::Value {
        &self.document
    }
}

#[derive(Debug, Error)]
pub enum ProjectManifestError {
    #[error("cannot read project manifest '{path}': {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse project manifest '{path}': {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "unsafe project manifest link or special file at '{0}'; expected one unique regular file"
    )]
    NonRegular(PathBuf),
}

/// Load the closest ancestor `ntnt.toml` for a file or directory.
pub fn load_project_manifest(
    start: impl AsRef<Path>,
) -> Result<Option<ProjectManifestDocument>, ProjectManifestError> {
    let start = start.as_ref();
    let directory = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    for ancestor in directory.ancestors() {
        let path = ancestor.join(crate::project::PROJECT_MANIFEST_NAME);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || has_multiple_hardlinks(&path, &metadata) =>
            {
                return Err(ProjectManifestError::NonRegular(path));
            }
            Ok(_) => return load_project_manifest_file(path).map(Some),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(ProjectManifestError::Read { path, source }),
        }
    }
    Ok(None)
}

/// Read and parse one exact project manifest without ancestor fallback.
pub fn load_project_manifest_file(
    path: impl AsRef<Path>,
) -> Result<ProjectManifestDocument, ProjectManifestError> {
    let path = path.as_ref();
    let content = std::fs::read_to_string(path).map_err(|source| ProjectManifestError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let document =
        content
            .parse::<toml::Value>()
            .map_err(|source| ProjectManifestError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let root = parent
        .canonicalize()
        .map_err(|source| ProjectManifestError::Read {
            path: parent.to_path_buf(),
            source,
        })?;
    let path = root.join(path.file_name().unwrap_or_default());
    Ok(ProjectManifestDocument {
        path,
        root,
        document,
    })
}

#[cfg(unix)]
pub(crate) fn has_multiple_hardlinks(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

#[cfg(windows)]
pub(crate) fn has_multiple_hardlinks(path: &Path, _metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let Ok(file) = std::fs::File::open(path) else {
        return true;
    };
    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    succeeded == 0 || information.nNumberOfLinks > 1
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn has_multiple_hardlinks(_path: &Path, _metadata: &std::fs::Metadata) -> bool {
    // These platforms expose no supported link-count API here. Treat an
    // unverifiable identity as unsafe rather than silently disabling the
    // manifest and resource hardlink guards.
    true
}

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
