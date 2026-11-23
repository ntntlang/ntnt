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
fn invalid_database_diagnostic_identifies_key_without_echoing_values() {
    let root = project();
    let manifest = root.path().join("ntnt.toml");
    let original = std::fs::read_to_string(&manifest).unwrap();
    for (extra, key) in [
        (
            "\n[databases]\nanalytics = 'credential_should_not_leak'\n",
            "databases.\"analytics\"",
        ),
        (
            "\n[databases.\"bad\\nname\"]\nvalue = 'credential_should_not_leak'\n",
            "databases.\"bad\\nname\"",
        ),
    ] {
        std::fs::write(&manifest, format!("{original}{extra}")).unwrap();
        let report = invoke(root.path(), &["--database", "app"]);
        let diagnostic = &report["diagnostics"][0];
        assert_eq!(diagnostic["code"], "DB_CONFIG");
        let message = diagnostic["message"].as_str().unwrap();
        assert!(
            message.contains(key),
            "missing offending database key: {message}"
        );
        assert!(!message.contains("credential_should_not_leak"));
        assert!(!message.contains('\n'));
    }
    let long_name = "x".repeat(10_000);
    std::fs::write(
        &manifest,
        format!("{original}\n[databases.\"{long_name}\"]\nvalue = 'credential_should_not_leak'\n"),
    )
    .unwrap();
    let report = invoke(root.path(), &["--database", "app"]);
    let message = report["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains('…'));
    assert!(
        message.len() < 512,
        "malformed key diagnostics must stay bounded"
    );
    assert!(!message.contains("credential_should_not_leak"));
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

/// `ntnt db status <args> --json` without secrets; it never passes here.
fn status(root: &std::path::Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "status", "--json"])
        .args(args)
        .env("NTNT_ENV", "development")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env_remove("SELECTION_APP")
        .env_remove("SELECTION_ANALYTICS")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "expected a structured result: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    report["diagnostics"][0].clone()
}

/// #422 (cli-18): a mistyped --target or --database echoes the value and
/// lists the declared names (manifest keys, never secrets); the advice names
/// them too, and following it selects a declared one.
#[test]
fn unknown_selections_list_the_declared_names() {
    let root = project();
    let manifest_advice = "declare [databases.<name>] with engine, schema";
    for target in ["dev", "Local"] {
        let d = status(root.path(), &["--target", target]);
        assert_eq!(d["code"], "DB_CONFIG", "{d}");
        assert_eq!(
            d["message"],
            format!("Target {target} is not declared for database app; declared targets: local."),
            "{d}"
        );
        let next_action = d["next_action"].as_str().unwrap();
        assert!(next_action.contains("--target local"), "{d}");
        assert!(!next_action.contains(manifest_advice), "{d}");
        assert!(!d.to_string().contains("SELECTION_APP"), "{d}");
    }
    // A value a terminal could interpret is escaped.
    let d = status(root.path(), &["--target", "de\u{1b}[2Jv"]);
    let message = d["message"].as_str().unwrap();
    assert!(message.contains("\"de\\u{1b}[2Jv\""), "{d}");
    assert!(!message.contains('\u{1b}'), "{d}");
    // Following the advice: the declared target gets as far as its secret.
    assert_eq!(
        status(root.path(), &["--target", "local"])["code"],
        "DB_SECRET"
    );

    let d = status(root.path(), &["--target", "local", "--database", "ap"]);
    assert_eq!(d["code"], "DB_DATABASE_UNKNOWN", "{d}");
    assert_eq!(
        d["message"],
        "Database ap is not declared; declared databases: app. No other database was selected.",
        "{d}"
    );
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("--database app"),
        "{d}"
    );

    // With two databases, --database is required and the choices are listed.
    let manifest = root.path().join("ntnt.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(
        "\n[databases.analytics]\nengine = \"postgresql\"\nschema = \"analytics\"\nmigrations_dir = \"not-provisioned\"\n[databases.analytics.targets.local]\nconnection_secret = \"SELECTION_ANALYTICS\"\ndatabase = \"analytics_db\"\nrole = \"analytics_role\"\n[secrets.SELECTION_ANALYTICS]\n",
    );
    std::fs::write(&manifest, text).unwrap();
    let d = status(root.path(), &["--target", "local"]);
    assert_eq!(d["code"], "DB_DATABASE_REQUIRED", "{d}");
    assert_eq!(
        d["message"],
        "Several logical databases are declared (analytics, app); select one with --database before choosing its target.",
        "{d}"
    );
    let next_action = d["next_action"].as_str().unwrap();
    assert!(
        next_action.contains("--database analytics") && next_action.contains("--database app"),
        "{d}"
    );
    let d = status(root.path(), &["--target", "local", "--database", "app"]);
    assert_eq!(d["code"], "DB_SECRET", "{d}");
}
