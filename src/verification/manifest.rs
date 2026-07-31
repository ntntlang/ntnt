//! Versioned project verification-manifest parsing.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::config::{has_multiple_hardlinks, load_project_manifest_file, ProjectManifestError};
use crate::project::ProjectRoot;

pub const VERIFICATION_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileClass {
    Application,
    Intent,
    Verification,
    Support,
    ProductAssets,
    Migrations,
    ProjectMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    path: PathBuf,
    class: FileClass,
}

impl ManifestFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn class(&self) -> FileClass {
        self.class
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVerificationManifest {
    version: u32,
    #[serde(default)]
    files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationManifest {
    project_root: PathBuf,
    version: u32,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error(transparent)]
    Project(#[from] ProjectManifestError),
    #[error("invalid [verification] manifest in '{path}': {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("unsupported verification manifest version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("verification resource '{0}' must be a confined relative path")]
    PathEscape(PathBuf),
    #[error("duplicate verification resource '{0}'")]
    DuplicateResource(PathBuf),
    #[error(
        "verification resource '{path}' has overlapping file classes {first:?} and {second:?}"
    )]
    OverlappingClasses {
        path: PathBuf,
        first: FileClass,
        second: FileClass,
    },
    #[error("verification resource '{path}' does not classify as {class:?}")]
    Unclassified { path: PathBuf, class: FileClass },
    #[error("verification resource '{0}' creates build-output ambiguity")]
    BuildOutput(PathBuf),
    #[error("verification resource '{0}' must be an existing regular file")]
    NotAFile(PathBuf),
    #[error("verification resource '{0}' is an unsafe symlink escape")]
    SymlinkEscape(PathBuf),
    #[error("verification resource '{0}' is an unsafe hardlink")]
    Hardlink(PathBuf),
    #[error("project manifest identity changed at '{0}'")]
    ManifestIdentity(PathBuf),
}

pub(super) fn contains_build_output_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if matches!(name.to_str(), Some("target" | "build" | "dist")))
    })
}

impl VerificationManifest {
    pub fn load(root: &ProjectRoot) -> Result<Self, ManifestError> {
        let path = root.manifest_path();
        let metadata =
            std::fs::symlink_metadata(path).map_err(|source| ProjectManifestError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        if metadata.file_type().is_symlink() || has_multiple_hardlinks(path, &metadata) {
            return Err(ManifestError::ManifestIdentity(path.to_path_buf()));
        }
        let loaded = load_project_manifest_file(path)?;
        if loaded.root() != root.path() || loaded.path() != root.manifest_path() {
            return Err(ManifestError::ManifestIdentity(path.to_path_buf()));
        }
        let document = loaded.document();
        let verification =
            document
                .get("verification")
                .cloned()
                .ok_or_else(|| ManifestError::Invalid {
                    path: path.to_path_buf(),
                    message: "missing [verification] table".to_string(),
                })?;
        let mut raw: RawVerificationManifest =
            verification
                .try_into()
                .map_err(|error| ManifestError::Invalid {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })?;
        if raw.version != VERIFICATION_MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedVersion {
                found: raw.version,
                expected: VERIFICATION_MANIFEST_VERSION,
            });
        }
        raw.files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut classes_by_path = HashMap::with_capacity(raw.files.len());
        for file in &raw.files {
            let lexical_path = file.path.to_string_lossy();
            let lexical_components = lexical_path.split(['/', '\\']).collect::<Vec<_>>();
            let windows_drive_prefix = lexical_components.first().is_some_and(|component| {
                let bytes = component.as_bytes();
                bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
            });
            let has_non_normal_lexical_component = windows_drive_prefix
                || lexical_components
                    .iter()
                    .any(|component| component.is_empty() || matches!(*component, "." | ".."));
            if file.path.as_os_str().is_empty()
                || file.path.is_absolute()
                || has_non_normal_lexical_component
                || file
                    .path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(ManifestError::PathEscape(file.path.clone()));
            }
            if contains_build_output_component(&file.path) {
                return Err(ManifestError::BuildOutput(file.path.clone()));
            }
            let resource_path = root.path().join(&file.path);
            let metadata = std::fs::metadata(&resource_path)
                .map_err(|_| ManifestError::NotAFile(file.path.clone()))?;
            if !metadata.is_file() {
                return Err(ManifestError::NotAFile(file.path.clone()));
            }
            if has_multiple_hardlinks(&resource_path, &metadata) {
                return Err(ManifestError::Hardlink(file.path.clone()));
            }
            let canonical_resource = resource_path
                .canonicalize()
                .map_err(|_| ManifestError::NotAFile(file.path.clone()))?;
            if !canonical_resource.starts_with(root.path()) {
                return Err(ManifestError::SymlinkEscape(file.path.clone()));
            }
            let canonical_relative = canonical_resource
                .strip_prefix(root.path())
                .expect("confined resource must remain under project root");
            if contains_build_output_component(canonical_relative) {
                return Err(ManifestError::BuildOutput(file.path.clone()));
            }
            if let Some(first) = classes_by_path.insert(file.path.clone(), file.class) {
                return if first == file.class {
                    Err(ManifestError::DuplicateResource(file.path.clone()))
                } else {
                    Err(ManifestError::OverlappingClasses {
                        path: file.path.clone(),
                        first,
                        second: file.class,
                    })
                };
            }
            let extension = file
                .path
                .extension()
                .and_then(|extension| extension.to_str());
            match extension {
                Some("intent") if file.class != FileClass::Intent => {
                    return Err(ManifestError::OverlappingClasses {
                        path: file.path.clone(),
                        first: FileClass::Intent,
                        second: file.class,
                    });
                }
                Some("tnt")
                    if !matches!(
                        file.class,
                        FileClass::Application | FileClass::Verification | FileClass::Support
                    ) =>
                {
                    return Err(ManifestError::Unclassified {
                        path: file.path.clone(),
                        class: file.class,
                    });
                }
                Some("intent" | "tnt") => {}
                _ if matches!(
                    file.class,
                    FileClass::Application
                        | FileClass::Intent
                        | FileClass::Verification
                        | FileClass::Support
                ) =>
                {
                    return Err(ManifestError::Unclassified {
                        path: file.path.clone(),
                        class: file.class,
                    });
                }
                _ => {}
            }
        }
        Ok(Self {
            project_root: root.path().to_path_buf(),
            version: raw.version,
            files: raw.files,
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn files(&self) -> &[ManifestFile] {
        &self.files
    }
}
