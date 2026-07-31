//! Deterministic project resource discovery.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::has_multiple_hardlinks;
use crate::project::ProjectRoot;

use super::manifest::{contains_build_output_component, FileClass, VerificationManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    relative_path: PathBuf,
    canonical_path: PathBuf,
    class: FileClass,
}

impl DiscoveredFile {
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn class(&self) -> FileClass {
        self.class
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiscovery {
    files: Vec<DiscoveredFile>,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("cannot discover verification resources at '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("ambiguous project root: nested project manifest '{0}'")]
    AmbiguousRoot(PathBuf),
    #[error("discovered path '{0}' is an unsafe symlink escape")]
    SymlinkEscape(PathBuf),
    #[error("discovered path '{0}' is an unsafe hardlink")]
    Hardlink(PathBuf),
    #[error("discovered path '{0}' has build-output ambiguity")]
    BuildOutput(PathBuf),
    #[error("discovered paths '{first}' and '{second}' are a duplicate resource")]
    DuplicateResource { first: PathBuf, second: PathBuf },
    #[error("automatically discovered resource '{0}' is also declared in the manifest (duplicate resource)")]
    AutomaticManifestOverlap(PathBuf),
    #[error("verification manifest belongs to different project root '{manifest_root}', expected '{requested_root}'")]
    ManifestRootMismatch {
        manifest_root: PathBuf,
        requested_root: PathBuf,
    },
}

impl ProjectDiscovery {
    pub fn discover(
        root: &ProjectRoot,
        manifest: &VerificationManifest,
    ) -> Result<Self, DiscoveryError> {
        if manifest.project_root() != root.path() {
            return Err(DiscoveryError::ManifestRootMismatch {
                manifest_root: manifest.project_root().to_path_buf(),
                requested_root: root.path().to_path_buf(),
            });
        }
        let mut classified = BTreeMap::new();
        collect_automatic(root.path(), root.path(), &mut classified)?;
        for file in manifest.files() {
            if classified.contains_key(file.path()) {
                return Err(DiscoveryError::AutomaticManifestOverlap(
                    file.path().to_path_buf(),
                ));
            }
            classified.insert(file.path().to_path_buf(), file.class());
        }

        let mut files = Vec::with_capacity(classified.len());
        let mut relative_by_canonical = BTreeMap::new();
        for (relative_path, class) in classified {
            let canonical_path =
                root.path()
                    .join(&relative_path)
                    .canonicalize()
                    .map_err(|source| DiscoveryError::Io {
                        path: relative_path.clone(),
                        source,
                    })?;
            if let Some(first) =
                relative_by_canonical.insert(canonical_path.clone(), relative_path.clone())
            {
                return Err(DiscoveryError::DuplicateResource {
                    first,
                    second: relative_path,
                });
            }
            files.push(DiscoveredFile {
                relative_path,
                canonical_path,
                class,
            });
        }
        Ok(Self { files })
    }

    pub fn files(&self) -> &[DiscoveredFile] {
        &self.files
    }
}

fn collect_automatic(
    root: &Path,
    directory: &Path,
    classified: &mut BTreeMap<PathBuf, FileClass>,
) -> Result<(), DiscoveryError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|source| DiscoveryError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| DiscoveryError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("discovery path must remain beneath root")
            .to_path_buf();
        if relative != Path::new("ntnt.toml") && entry.file_name() == "ntnt.toml" {
            return Err(DiscoveryError::AmbiguousRoot(relative));
        }
        let file_type = entry.file_type().map_err(|source| DiscoveryError::Io {
            path: path.clone(),
            source,
        })?;
        let resolved_is_file = if file_type.is_symlink() {
            let canonical = path.canonicalize().map_err(|source| DiscoveryError::Io {
                path: path.clone(),
                source,
            })?;
            if !canonical.starts_with(root) {
                let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                return Err(DiscoveryError::SymlinkEscape(relative));
            }
            let canonical_relative = canonical
                .strip_prefix(root)
                .expect("confined discovery path must remain beneath root");
            if contains_build_output_component(canonical_relative) {
                return Err(DiscoveryError::BuildOutput(relative));
            }
            canonical.is_file()
        } else {
            file_type.is_file()
        };
        if file_type.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.')
                || matches!(name.as_ref(), "node_modules" | "target" | "build" | "dist")
            {
                continue;
            }
            collect_automatic(root, &path, classified)?;
        } else if resolved_is_file {
            let class = match path.extension().and_then(|extension| extension.to_str()) {
                Some("intent") => Some(FileClass::Intent),
                _ => None,
            };
            if let Some(class) = class {
                let metadata = path.metadata().map_err(|source| DiscoveryError::Io {
                    path: relative.clone(),
                    source,
                })?;
                if has_multiple_hardlinks(&path, &metadata) {
                    return Err(DiscoveryError::Hardlink(relative));
                }
                classified.insert(relative, class);
            }
        }
    }
    Ok(())
}
