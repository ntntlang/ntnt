//! Canonical NTNT project-root resolution.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::{has_multiple_hardlinks, load_project_manifest, ProjectManifestError};

pub const PROJECT_MANIFEST_NAME: &str = "ntnt.toml";

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("cannot resolve project path '{path}': {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("no {PROJECT_MANIFEST_NAME} project manifest found from '{0}'")]
    ManifestNotFound(PathBuf),
    #[error("ambiguous project roots: '{first}' and '{second}'")]
    AmbiguousRoots { first: PathBuf, second: PathBuf },
    #[error("unsafe project manifest link '{0}'")]
    UnsafeManifestLink(PathBuf),
    #[error(transparent)]
    Manifest(#[from] ProjectManifestError),
}

/// A project root and manifest represented by canonical absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot {
    path: PathBuf,
    manifest_path: PathBuf,
}

impl ProjectRoot {
    /// Resolve the closest ancestor project manifest from a file or directory.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let requested = start.as_ref();
        let lexical_manifest = load_project_manifest(requested)?;
        let canonical = requested
            .canonicalize()
            .map_err(|source| ProjectError::Canonicalize {
                path: requested.to_path_buf(),
                source,
            })?;
        let canonical_manifest = load_project_manifest(&canonical)?;
        let canonical_start = if canonical.is_dir() {
            canonical.as_path()
        } else {
            canonical.parent().unwrap_or(&canonical)
        };
        let selected = match (lexical_manifest, canonical_manifest) {
            (Some(lexical), Some(canonical)) if lexical.root() == canonical.root() => canonical,
            (Some(lexical), Some(canonical)) => {
                return Err(ProjectError::AmbiguousRoots {
                    first: lexical.root().to_path_buf(),
                    second: canonical.root().to_path_buf(),
                });
            }
            (Some(lexical), None) => {
                return Err(ProjectError::AmbiguousRoots {
                    first: lexical.root().to_path_buf(),
                    second: canonical_start.to_path_buf(),
                });
            }
            (None, Some(canonical_manifest)) => {
                return Err(ProjectError::AmbiguousRoots {
                    first: requested.to_path_buf(),
                    second: canonical_manifest.root().to_path_buf(),
                });
            }
            (None, None) => {
                return Err(ProjectError::ManifestNotFound(requested.to_path_buf()));
            }
        };
        let manifest_metadata = std::fs::symlink_metadata(selected.path()).map_err(|source| {
            ProjectManifestError::Read {
                path: selected.path().to_path_buf(),
                source,
            }
        })?;
        if manifest_metadata.file_type().is_symlink()
            || has_multiple_hardlinks(selected.path(), &manifest_metadata)
        {
            return Err(ProjectError::UnsafeManifestLink(
                selected.path().to_path_buf(),
            ));
        }
        let path = selected.root().to_path_buf();

        Ok(Self {
            manifest_path: selected.path().to_path_buf(),
            path,
        })
    }

    /// Require every supplied resource to resolve to the same canonical root.
    pub fn discover_all<I, P>(paths: I) -> Result<Self, ProjectError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut roots = paths.into_iter().map(Self::discover);
        let first = roots
            .next()
            .ok_or_else(|| ProjectError::ManifestNotFound(PathBuf::from(".")))??;
        for root in roots {
            let root = root?;
            if root != first {
                return Err(ProjectError::AmbiguousRoots {
                    first: first.path.clone(),
                    second: root.path,
                });
            }
        }
        Ok(first)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }
}
