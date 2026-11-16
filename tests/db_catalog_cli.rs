use std::process::Command;

fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("migrations")).unwrap();
    std::fs::write(
        root.path().join("ntnt.toml"),
        r#"
[databases.app]
engine = "postgresql"
migrations_dir = "migrations"
schema = "app"
[databases.app.targets.local]
connection_secret = "DB_TARGET"
database = "target"
role = "target_role"
[secrets.DB_TARGET]
"#,
    )
    .unwrap();
    root
}
fn inspect(root: &std::path::Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "status", "--target", "local", "--json"])
        .current_dir(root)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn duplicate_numeric_ids_are_rejected_before_connecting() {
    let root = project();
    for name in ["0001_one.sql", "01_two.sql"] {
        std::fs::write(
            root.path().join("migrations").join(name),
            "CREATE TABLE app.t (id int);",
        )
        .unwrap();
    }
    assert_eq!(
        inspect(root.path())["diagnostics"][0]["code"],
        "DB_DUPLICATE_ID"
    );
}

#[test]
fn db_reports_manifest_errors_as_versioned_json() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "status", "--target", "local", "--json"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&output.stderr)));
    assert_eq!(report["format_version"], 1);
    assert_eq!(report["outcome"], "unknown");
    assert_eq!(report["diagnostics"][0]["code"], "DB_INPUT_IO");
}

#[test]
fn stalled_authentication_is_bounded_and_credentials_are_redacted() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        let _ = stop_rx.recv_timeout(std::time::Duration::from_secs(12));
    });
    let root = project();
    let secret=format!("host=127.0.0.1 port={port} user=target_role dbname=target password=ntnt245_private_canary sslmode=disable");
    let start = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "status", "--target", "local", "--json"])
        .env("DB_TARGET", secret)
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(root.path())
        .output()
        .unwrap();
    stop_tx.send(()).unwrap();
    server.join().unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("ntnt245_private_canary"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("ntnt245_private_canary"));
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["diagnostics"][0]["code"], "DB_TIMEOUT");
    assert!(!output.status.success());
}

#[test]
fn unsupported_metadata_and_unsafe_paths_are_explicit_errors() {
    let root = project();
    std::fs::write(
        root.path().join("migrations/1_first.sql"),
        "CREATE TABLE app.t(id INT);",
    )
    .unwrap();
    for metadata in [
        "requires = []",
        "mode = 'non_transactional'",
        "[checks]\nquery='SELECT 1'",
    ] {
        std::fs::write(root.path().join("migrations/1_first.toml"), metadata).unwrap();
        assert_eq!(
            inspect(root.path())["diagnostics"][0]["code"],
            "DB_METADATA_UNSUPPORTED"
        );
    }
    std::fs::remove_file(root.path().join("migrations/1_first.toml")).unwrap();
    let manifest = std::fs::read_to_string(root.path().join("ntnt.toml")).unwrap();
    for path in [
        "../outside",
        "/absolute",
        "C:drive",
        "migrations//child",
        "migrations/./child",
    ] {
        std::fs::write(
            root.path().join("ntnt.toml"),
            manifest.replace(
                "migrations_dir = \"migrations\"",
                &format!("migrations_dir = {path:?}"),
            ),
        )
        .unwrap();
        assert_eq!(inspect(root.path())["diagnostics"][0]["code"], "DB_CONFIG");
    }
}

#[test]
fn configuration_must_be_tables_not_positional_arrays() {
    let root = project();
    let manifest = std::fs::read_to_string(root.path().join("ntnt.toml")).unwrap();
    assert!(manifest.contains("[databases.app]\n"), "{manifest}");
    let first_diagnostic = |manifest: String| {
        std::fs::write(root.path().join("ntnt.toml"), manifest).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .args(["db", "status", "--target", "local", "--json"])
            .env_remove("DB_TARGET")
            .current_dir(root.path())
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        report["diagnostics"][0].clone()
    };
    // Serde would read these arrays positionally as struct fields.
    for limits in ["[]", "[90]", "[1, 300]"] {
        let d = first_diagnostic(manifest.replacen(
            "[databases.app]\n",
            &format!("[databases.app]\nlimits = {limits}\n"),
            1,
        ));
        assert_eq!(d["code"], "DB_CONFIG", "{limits}: {d}");
        assert!(
            d["message"]
                .as_str()
                .unwrap()
                .contains("[databases.app.limits] must be a table"),
            "{limits}: {d}"
        );
    }
    // A target entry is a struct too: as an array it would otherwise connect.
    let target = "[databases.app.targets.local]\nconnection_secret = \"DB_TARGET\"\ndatabase = \"target\"\nrole = \"target_role\"\n";
    assert!(manifest.contains(target), "{manifest}");
    let d = first_diagnostic(manifest.replacen(target, "", 1).replacen(
        "[databases.app]\n",
        "[databases.app]\ntargets = { local = [\"DB_TARGET\", \"target\", \"target_role\"] }\n",
        1,
    ));
    assert_eq!(d["code"], "DB_CONFIG", "{d}");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("must be TOML tables"),
        "{d}"
    );
}

#[test]
fn time_limits_are_validated_before_connecting() {
    let root = project();
    let manifest = std::fs::read_to_string(root.path().join("ntnt.toml")).unwrap();
    let first_diagnostic = |limits: &str| {
        std::fs::write(
            root.path().join("ntnt.toml"),
            format!("{manifest}[databases.app.limits]\n{limits}\n"),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .args(["db", "status", "--target", "local", "--json"])
            .env_remove("DB_TARGET")
            .current_dir(root.path())
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        report["diagnostics"][0].clone()
    };
    // Valid limits pass configuration; the missing secret stops inspection.
    let d = first_diagnostic("statement_seconds = 86400\nmigrator_wait_seconds = 300");
    assert_eq!(d["code"], "DB_SECRET", "{d}");
    for (limits, field) in [
        ("statement_seconds = 0", "statement_seconds"),
        ("statement_seconds = 86401", "statement_seconds"),
        ("migrator_wait_seconds = 0", "migrator_wait_seconds"),
        ("migrator_wait_seconds = 301", "migrator_wait_seconds"),
        ("statement_seconds = 1.5", "statement_seconds"),
        (
            "statement_seconds = \"ntnt346_private_canary\"",
            "statement_seconds",
        ),
        ("deadline_seconds = 600", "migrator_wait_seconds"),
    ] {
        let d = first_diagnostic(limits);
        assert_eq!(d["code"], "DB_CONFIG", "{limits}: {d}");
        let message = d["message"].as_str().unwrap();
        assert!(
            message.contains("[databases.app.limits]") && message.contains(field),
            "{limits}: {d}"
        );
        assert!(!d.to_string().contains("ntnt346_private_canary"), "{d}");
        assert!(
            d["next_action"]
                .as_str()
                .unwrap()
                .contains("migrator_wait_seconds"),
            "{d}"
        );
    }
}

#[test]
fn original_source_migrate_command_remains_available() {
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["migrate", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--dry-run"));
    assert!(!Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "migrate"])
        .output()
        .unwrap()
        .status
        .success());
}

#[test]
fn output_never_replaces_an_input_and_reports_a_structured_error() {
    let root = project();
    let original = std::fs::read(root.path().join("ntnt.toml")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args([
            "db",
            "plan",
            "--target",
            "local",
            "--json",
            "--out",
            "ntnt.toml",
        ])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(root.path().join("ntnt.toml")).unwrap(),
        original
    );
    let r: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "DB_OUTPUT"));
}
