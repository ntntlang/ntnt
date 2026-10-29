//! Logical database selection must be explicit when the manifest is ambiguous.
use serde_json::Value;
use std::process::Command;

fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("app-migrations")).unwrap();
    std::fs::write(
        root.path().join("ntnt.toml"),
        r#"
[databases.app]
engine = "postgresql"
schema = "app"
migrations_dir = "app-migrations"
[databases.app.targets.local]
connection_secret = "SELECTION_APP"
database = "app_db"
role = "app_role"
[secrets.SELECTION_APP]
"#,
    )
    .unwrap();
    root
}

fn invoke(root: &std::path::Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "status", "--target", "local", "--json"])
        .args(args)
        .env("NTNT_ENV", "development")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env_remove("SELECTION_APP")
        .env_remove("SELECTION_ANALYTICS")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "expected structured selection result: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn explicit_selection_ignores_unselected_backend_and_unavailable_files() {
    let root = project();
    let manifest = root.path().join("ntnt.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    // A future backend's fields must not be interpreted by the selected adapter.
    text.push_str("\n[databases.analytics]\nengine = 'future_adapter'\npath = 'not-provisioned'\nconnection_secret = 'UNDECLARED_OTHER_SECRET'\n");
    std::fs::write(manifest, text).unwrap();
    assert_eq!(
        invoke(root.path(), &["--database", "app"])["diagnostics"][0]["code"],
        "DB_SECRET"
    );
    assert_eq!(
        invoke(root.path(), &["--database", "missing"])["diagnostics"][0]["code"],
        "DB_DATABASE_UNKNOWN"
    );
}

#[test]
fn prototype_or_mixed_manifest_forms_are_rejected_without_guessing() {
    let root = project();
    let manifest = root.path().join("ntnt.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    for value in [
        text.replace("databases.app", "database"),
        format!("{text}\n[database]\nengine = 'postgresql'\n"),
    ] {
        std::fs::write(&manifest, value).unwrap();
        let report = invoke(root.path(), &["--database", "app"]);
        assert_eq!(report["diagnostics"][0]["code"], "DB_CONFIG");
        assert!(report["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("prototype"));
    }
}

#[test]
fn target_is_required_even_for_a_single_database() {
    let root = project();
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "status", "--database", "app", "--json"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--target"));
}

#[test]
fn single_named_database_is_selected_without_a_database_flag() {
    let root = project();
    // Reaching secret lookup proves the named database configuration and its
    // migration directory were selected; this test never connects to a server.
    let report = invoke(root.path(), &[]);
    assert_eq!(report["diagnostics"][0]["code"], "DB_SECRET");
}

#[test]
fn multiple_databases_require_selection_before_secrets_or_filesystem_lookup() {
    let root = project();
    let manifest = root.path().join("ntnt.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(
        r#"
[databases.analytics]
engine = "postgresql"
schema = "analytics"
migrations_dir = "not-provisioned"
[databases.analytics.targets.local]
connection_secret = "SELECTION_ANALYTICS"
database = "analytics_db"
role = "analytics_role"
[secrets.SELECTION_ANALYTICS]
"#,
    );
    std::fs::write(manifest, text).unwrap();
    let report = invoke(root.path(), &[]);
    assert_eq!(report["diagnostics"][0]["code"], "DB_DATABASE_REQUIRED");
}
