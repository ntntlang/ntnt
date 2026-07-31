use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use ntnt::config::load_project_manifest;
use ntnt::project::ProjectRoot;
use ntnt::verification::{
    FileClass, ProjectDiscovery, VerificationManifest, VERIFICATION_MANIFEST_VERSION,
};

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

struct TempProject {
    path: PathBuf,
}

impl TempProject {
    fn new(manifest: &str) -> Self {
        let sequence = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ntnt-verification-manifest-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("ntnt.toml"), manifest).unwrap();
        Self { path }
    }

    fn root(&self) -> ProjectRoot {
        ProjectRoot::discover(&self.path).unwrap()
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

fn project_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/verification/projects")
        .join(name)
}

#[test]
fn nested_source_resolves_to_canonical_manifest_root() {
    let fixture = project_fixture("canonical");
    let root = ProjectRoot::discover(fixture.join("src/nested/app.tnt")).unwrap();

    assert_eq!(root.path(), fixture.canonicalize().unwrap());
    assert_eq!(root.manifest_path(), root.path().join("ntnt.toml"));
}

#[test]
fn resources_from_distinct_project_roots_are_ambiguous() {
    let first = project_fixture("canonical").join("src/nested/app.tnt");
    let second = project_fixture("other").join("other.tnt");

    let error = ProjectRoot::discover_all([first, second]).unwrap_err();

    assert!(
        error.to_string().contains("ambiguous project roots"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn ambiguous_directory_roots_report_the_canonical_directory() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let outside = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    std::fs::remove_file(outside.path.join("ntnt.toml")).unwrap();
    let alias = project.path.join("outside-alias");
    symlink(&outside.path, &alias).unwrap();

    let error = ProjectRoot::discover(&alias).unwrap_err();
    let canonical_outside = outside.path.canonicalize().unwrap();

    assert!(
        error
            .to_string()
            .contains(&canonical_outside.to_string_lossy().to_string()),
        "{error}"
    );
}

#[test]
fn versioned_manifest_loads_exhaustive_file_classes() {
    let root = ProjectRoot::discover(project_fixture("canonical")).unwrap();
    let manifest = VerificationManifest::load(&root).unwrap();

    assert_eq!(manifest.version(), VERIFICATION_MANIFEST_VERSION);
    assert_eq!(manifest.files().len(), 3);
    assert_eq!(manifest.files()[0].class(), FileClass::ProductAssets);
    assert_eq!(manifest.files()[1].class(), FileClass::Application);
    assert_eq!(manifest.files()[2].class(), FileClass::Verification);
}

#[test]
fn configured_ntnt_verification_files_have_their_own_class() {
    let project = TempProject::new(
        "[verification]\nversion = 1\n[[verification.files]]\npath = 'verification/smoke.tnt'\nclass = 'verification'\n",
    );
    project.write("verification/smoke.tnt", "fn smoke() {}\n");

    let manifest = VerificationManifest::load(&project.root()).unwrap();

    assert_eq!(manifest.files().len(), 1);
    assert_eq!(manifest.files()[0].class(), FileClass::Verification);
}

#[test]
fn manifest_supports_the_exhaustive_file_class_vocabulary() {
    let project = TempProject::new(
        "[verification]\nversion = 1\n\
         [[verification.files]]\npath = 'app.tnt'\nclass = 'application'\n\
         [[verification.files]]\npath = 'spec.intent'\nclass = 'intent'\n\
         [[verification.files]]\npath = 'verification/check.tnt'\nclass = 'verification'\n\
         [[verification.files]]\npath = 'tools/support.tnt'\nclass = 'support'\n\
         [[verification.files]]\npath = 'public/app.js'\nclass = 'product-assets'\n\
         [[verification.files]]\npath = 'migrations/schema.sql'\nclass = 'migrations'\n\
         [[verification.files]]\npath = 'README.md'\nclass = 'project-metadata'\n",
    );
    for path in [
        "app.tnt",
        "spec.intent",
        "verification/check.tnt",
        "tools/support.tnt",
        "public/app.js",
        "migrations/schema.sql",
        "README.md",
    ] {
        project.write(path, "fixture\n");
    }

    let manifest = VerificationManifest::load(&project.root()).unwrap();

    for expected in [
        FileClass::Application,
        FileClass::Intent,
        FileClass::Verification,
        FileClass::Support,
        FileClass::ProductAssets,
        FileClass::Migrations,
        FileClass::ProjectMetadata,
    ] {
        assert!(
            manifest.files().iter().any(|file| file.class() == expected),
            "missing {expected:?}"
        );
    }
}

#[test]
fn manifest_rejects_unknown_fields() {
    for manifest in [
        "[verification]\nversion = 1\nunknown = true\nfiles = []\n",
        "[verification]\nversion = 1\n[[verification.files]]\npath = 'check.rs'\nclass = 'verification'\nunknown = true\n",
    ] {
        let project = TempProject::new(manifest);
        let error = VerificationManifest::load(&project.root()).unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}

#[test]
fn manifest_loading_does_not_retarget_to_an_outer_project() {
    let outer = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    outer.write(
        "inner/ntnt.toml",
        "[verification]\nversion = 1\nfiles = []\n",
    );
    let source = outer.write("inner/app.tnt", "print(\"inner\")\n");
    let root = ProjectRoot::discover(source).unwrap();
    std::fs::remove_file(root.manifest_path()).unwrap();

    let error = VerificationManifest::load(&root).unwrap_err();

    assert!(
        error.to_string().contains("manifest") || error.to_string().contains("read"),
        "{error}"
    );
}

#[test]
fn manifest_rejects_unsupported_versions() {
    let project = TempProject::new("[verification]\nversion = 2\nfiles = []\n");

    let error = VerificationManifest::load(&project.root()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unsupported verification manifest version 2"),
        "{error}"
    );
}

#[test]
fn manifest_paths_cannot_traverse_or_escape_the_project_root() {
    let absolute = std::env::temp_dir().join("outside-verification.rs");
    for path in [
        "../outside.rs".to_string(),
        "checks/./smoke.tnt".to_string(),
        "checks//smoke.tnt".to_string(),
        r"C:\outside\smoke.tnt".to_string(),
        absolute.display().to_string(),
    ] {
        let manifest = format!(
            "[verification]\nversion = 1\n[[verification.files]]\npath = {path:?}\nclass = 'verification'\n"
        );
        let project = TempProject::new(&manifest);

        let error = VerificationManifest::load(&project.root()).unwrap_err();

        assert!(
            error.to_string().contains("confined relative path"),
            "{error}"
        );
    }
}

#[test]
fn manifest_rejects_duplicate_and_overlapping_resources() {
    for (second_class, expected) in [
        ("verification", "duplicate verification resource"),
        ("support", "overlapping file classes"),
    ] {
        let manifest = format!(
            "[verification]\nversion = 1\n[[verification.files]]\npath = 'verification/smoke.tnt'\nclass = 'verification'\n[[verification.files]]\npath = 'verification/smoke.tnt'\nclass = '{second_class}'\n"
        );
        let project = TempProject::new(&manifest);
        project.write("verification/smoke.tnt", "fn smoke() {}\n");

        let error = VerificationManifest::load(&project.root()).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn automatic_file_classes_cannot_overlap_or_be_unclassified() {
    for (path, class, expected) in [
        (
            "specs/app.intent",
            "verification",
            "overlapping file classes",
        ),
        ("data/request.json", "intent", "does not classify"),
        ("migrations/step.tnt", "migrations", "does not classify"),
    ] {
        let manifest = format!(
            "[verification]\nversion = 1\n[[verification.files]]\npath = '{path}'\nclass = '{class}'\n"
        );
        let project = TempProject::new(&manifest);
        project.write(path, "fixture\n");

        let error = VerificationManifest::load(&project.root()).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn build_output_resources_are_ambiguous() {
    for path in ["target/check.rs", "build/check.rs", "dist/check.rs"] {
        let manifest = format!(
            "[verification]\nversion = 1\n[[verification.files]]\npath = '{path}'\nclass = 'verification'\n"
        );
        let project = TempProject::new(&manifest);
        project.write(path, "fn check() {}\n");

        let error = VerificationManifest::load(&project.root()).unwrap_err();

        assert!(
            error.to_string().contains("build-output ambiguity"),
            "{error}"
        );
    }
}

#[cfg(unix)]
#[test]
fn configured_resources_cannot_alias_build_output_directories() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new(
        "[verification]\nversion = 1\n[[verification.files]]\npath = 'checks/generated.tnt'\nclass = 'verification'\n",
    );
    project.write("target/generated.tnt", "fn generated() {}\n");
    symlink(project.path.join("target"), project.path.join("checks")).unwrap();

    let error = VerificationManifest::load(&project.root()).unwrap_err();

    assert!(
        error.to_string().contains("build-output ambiguity"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn automatic_discovery_rejects_canonical_build_output_aliases() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let target = project.write("target/generated.intent", "Feature: Generated\n");
    std::fs::create_dir_all(project.path.join("specs")).unwrap();
    symlink(target, project.path.join("specs/alias.intent")).unwrap();
    let root = project.root();
    let manifest = VerificationManifest::load(&root).unwrap();

    let error = ProjectDiscovery::discover(&root, &manifest).unwrap_err();

    assert!(
        error.to_string().contains("build-output ambiguity"),
        "{error}"
    );
}

#[test]
fn configured_resources_must_be_existing_regular_files() {
    for create_directory in [false, true] {
        let project = TempProject::new(
            "[verification]\nversion = 1\n[[verification.files]]\npath = 'checks/missing.rs'\nclass = 'verification'\n",
        );
        if create_directory {
            std::fs::create_dir_all(project.path.join("checks/missing.rs")).unwrap();
        }

        let error = VerificationManifest::load(&project.root()).unwrap_err();

        assert!(
            error.to_string().contains("existing regular file"),
            "{error}"
        );
    }
}

#[cfg(unix)]
#[test]
fn configured_symlinks_cannot_escape_the_project_root() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new(
        "[verification]\nversion = 1\n[[verification.files]]\npath = 'checks/escape.rs'\nclass = 'verification'\n",
    );
    let outside = project.path.with_extension("outside.rs");
    std::fs::write(&outside, "fn outside() {}\n").unwrap();
    std::fs::create_dir_all(project.path.join("checks")).unwrap();
    symlink(&outside, project.path.join("checks/escape.rs")).unwrap();

    let error = VerificationManifest::load(&project.root()).unwrap_err();

    assert!(error.to_string().contains("symlink escape"), "{error}");
    std::fs::remove_file(outside).ok();
}

#[cfg(unix)]
#[test]
fn configured_hardlinks_are_rejected_as_unsafe() {
    let project = TempProject::new(
        "[verification]\nversion = 1\n[[verification.files]]\npath = 'checks/hardlink.rs'\nclass = 'verification'\n",
    );
    let outside = project.path.with_extension("outside-hardlink.rs");
    std::fs::write(&outside, "fn outside() {}\n").unwrap();
    std::fs::create_dir_all(project.path.join("checks")).unwrap();
    std::fs::hard_link(&outside, project.path.join("checks/hardlink.rs")).unwrap();

    let error = VerificationManifest::load(&project.root()).unwrap_err();

    assert!(error.to_string().contains("unsafe hardlink"), "{error}");
    std::fs::remove_file(outside).ok();
}

#[test]
fn discovery_finds_nested_intent_and_configured_files_in_path_order() {
    let root = ProjectRoot::discover(project_fixture("canonical")).unwrap();
    let manifest = VerificationManifest::load(&root).unwrap();

    let discovery = ProjectDiscovery::discover(&root, &manifest).unwrap();
    let files: Vec<_> = discovery
        .files()
        .iter()
        .map(|file| (file.relative_path().to_path_buf(), file.class()))
        .collect();

    assert_eq!(
        files,
        [
            (
                PathBuf::from("fixtures/request.json"),
                FileClass::ProductAssets
            ),
            (
                PathBuf::from("specs/nested/project.intent"),
                FileClass::Intent
            ),
            (PathBuf::from("src/nested/app.tnt"), FileClass::Application),
            (
                PathBuf::from("verification/smoke.tnt"),
                FileClass::Verification
            ),
        ]
    );
}

#[test]
fn automatic_and_configured_resources_cannot_overlap() {
    let project = TempProject::new(
        "[verification]\nversion = 1\n[[verification.files]]\npath = 'specs/app.intent'\nclass = 'intent'\n",
    );
    project.write("specs/app.intent", "Feature: App\n");
    let root = project.root();
    let manifest = VerificationManifest::load(&root).unwrap();

    let error = ProjectDiscovery::discover(&root, &manifest).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("automatically discovered resource 'specs/app.intent' is also declared"),
        "{error}"
    );
}

#[test]
fn manifests_cannot_be_reused_with_a_different_project_root() {
    let first = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    first.write("verification/check.tnt", "fn first() {}\n");
    let second = TempProject::new(
        "[verification]\nversion = 1\n[[verification.files]]\npath = 'verification/check.tnt'\nclass = 'verification'\n",
    );
    second.write("verification/check.tnt", "fn second() {}\n");
    let first_root = first.root();
    let second_manifest = VerificationManifest::load(&second.root()).unwrap();

    let error = ProjectDiscovery::discover(&first_root, &second_manifest).unwrap_err();

    assert!(
        error.to_string().contains("different project root"),
        "{error}"
    );
}

#[test]
fn nested_manifests_make_project_discovery_ambiguous() {
    let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    project.write(
        "nested/ntnt.toml",
        "[verification]\nversion = 1\nfiles = []\n",
    );
    project.write("nested/spec.intent", "Feature: Nested\n");
    let root = project.root();
    let manifest = VerificationManifest::load(&root).unwrap();

    let error = ProjectDiscovery::discover(&root, &manifest).unwrap_err();

    assert!(
        error.to_string().contains("ambiguous project root"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn automatic_discovery_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let outside = project.path.with_extension("outside.intent");
    std::fs::write(&outside, "Feature: Outside\n").unwrap();
    std::fs::create_dir_all(project.path.join("specs")).unwrap();
    symlink(&outside, project.path.join("specs/escape.intent")).unwrap();
    let root = project.root();
    let manifest = VerificationManifest::load(&root).unwrap();

    let error = ProjectDiscovery::discover(&root, &manifest).unwrap_err();

    assert!(error.to_string().contains("symlink escape"), "{error}");
    std::fs::remove_file(outside).ok();
}

#[cfg(unix)]
#[test]
fn automatic_discovery_rejects_unsafe_hardlinks() {
    let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let outside = project.path.with_extension("outside-hardlink.intent");
    std::fs::write(&outside, "Feature: Outside\n").unwrap();
    std::fs::create_dir_all(project.path.join("specs")).unwrap();
    std::fs::hard_link(&outside, project.path.join("specs/hardlink.intent")).unwrap();
    let root = project.root();
    let manifest = VerificationManifest::load(&root).unwrap();

    let error = ProjectDiscovery::discover(&root, &manifest).unwrap_err();

    assert!(error.to_string().contains("unsafe hardlink"), "{error}");
    std::fs::remove_file(outside).ok();
}

#[cfg(unix)]
#[test]
fn safe_symlink_aliases_are_duplicate_resources() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let target = project.write("specs/real.intent", "Feature: Real\n");
    symlink(&target, project.path.join("specs/alias.intent")).unwrap();
    let root = project.root();
    let manifest = VerificationManifest::load(&root).unwrap();

    let error = ProjectDiscovery::discover(&root, &manifest).unwrap_err();

    assert!(error.to_string().contains("duplicate resource"), "{error}");
}

#[test]
fn nested_non_regular_manifest_markers_are_ambiguous() {
    let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    std::fs::create_dir_all(project.path.join("nested/ntnt.toml")).unwrap();
    let root = project.root();
    let manifest = VerificationManifest::load(&root).unwrap();

    let error = ProjectDiscovery::discover(&root, &manifest).unwrap_err();

    assert!(
        error.to_string().contains("nested project manifest"),
        "{error}"
    );
}

#[test]
fn shared_manifest_loader_preserves_closest_ancestor_behavior() {
    let project = TempProject::new("[lint]\nstrict = false\n");
    project.write("nested/ntnt.toml", "[lint]\nstrict = true\n");
    let source = project.write("nested/src/app.tnt", "print(\"nested\")\n");

    let loaded = load_project_manifest(&source).unwrap().unwrap();

    assert_eq!(
        loaded.path(),
        project
            .path
            .join("nested/ntnt.toml")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        loaded
            .document()
            .get("lint")
            .and_then(|lint| lint.get("strict"))
            .and_then(toml::Value::as_bool),
        Some(true)
    );
}

#[test]
fn root_relative_cli_paths_still_load_the_project_manifest() {
    let project = TempProject::new("[lint]\nstrict = true\n");
    project.write("app.tnt", "fn identity(value) {\n    return value\n}\n");

    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["lint", "app.tnt"])
        .current_dir(&project.path)
        .output()
        .expect("run ntnt lint");

    assert!(
        !output.status.success(),
        "relative path bypassed strict manifest:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn non_regular_nearest_manifest_does_not_inherit_outer_configuration() {
    let project = TempProject::new("[lint]\nstrict = true\n");
    std::fs::create_dir_all(project.path.join("inner/ntnt.toml")).unwrap();
    let source = project.write("inner/src/app.tnt", "print(\"inner\")\n");

    assert!(load_project_manifest(source).is_err());
}

#[cfg(unix)]
#[test]
fn dangling_nearest_manifest_does_not_inherit_outer_configuration() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new("[lint]\nstrict = true\n");
    std::fs::create_dir_all(project.path.join("inner/src")).unwrap();
    symlink("missing.toml", project.path.join("inner/ntnt.toml")).unwrap();
    let source = project.write("inner/src/app.tnt", "print(\"inner\")\n");

    assert!(load_project_manifest(source).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_nearest_manifest_does_not_inherit_outer_configuration() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new("[lint]\nstrict = true\n");
    std::fs::create_dir_all(project.path.join("inner/src")).unwrap();
    symlink("../ntnt.toml", project.path.join("inner/ntnt.toml")).unwrap();
    let source = project.write("inner/src/app.tnt", "print(\"inner\")\n");

    assert!(load_project_manifest(source).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_sources_cannot_hide_an_ambiguous_project_root() {
    use std::os::unix::fs::symlink;

    let lexical = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let canonical = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let target = canonical.write("app.tnt", "print(\"canonical\")\n");
    let link = lexical.path.join("app.tnt");
    symlink(target, &link).unwrap();

    let error = ProjectRoot::discover(link).unwrap_err();

    assert!(
        error.to_string().contains("ambiguous project roots"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn lexical_and_canonical_project_resolution_must_both_exist() {
    use std::os::unix::fs::symlink;

    let lexical = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
    let outside = lexical.path.with_extension("outside-root");
    std::fs::create_dir_all(&outside).unwrap();
    let target = outside.join("app.tnt");
    std::fs::write(&target, "print(\"outside\")\n").unwrap();
    let link = lexical.path.join("app.tnt");
    symlink(&target, &link).unwrap();

    let error = ProjectRoot::discover(link).unwrap_err();

    assert!(
        error.to_string().contains("ambiguous project roots"),
        "{error}"
    );
    std::fs::remove_dir_all(outside).ok();
}

#[cfg(unix)]
#[test]
fn project_manifest_links_are_unsafe() {
    use std::os::unix::fs::symlink;

    for hardlink in [false, true] {
        let project = TempProject::new("[verification]\nversion = 1\nfiles = []\n");
        let manifest = project.path.join("ntnt.toml");
        std::fs::remove_file(&manifest).unwrap();
        let outside = project.path.with_extension(if hardlink {
            "outside-hardlink.toml"
        } else {
            "outside-symlink.toml"
        });
        std::fs::write(&outside, "[verification]\nversion = 1\nfiles = []\n").unwrap();
        if hardlink {
            std::fs::hard_link(&outside, &manifest).unwrap();
        } else {
            symlink(&outside, &manifest).unwrap();
        }

        let error = ProjectRoot::discover(&project.path).unwrap_err();

        assert!(
            error.to_string().contains("unsafe project manifest link"),
            "{error}"
        );
        std::fs::remove_file(outside).ok();
    }
}
