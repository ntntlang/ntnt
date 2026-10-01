//! Multi-file fixture app (#260): an entry file with `lib/` helpers, `views/`
//! templates and partials, file-based `routes/`, and a `jobs/` directory loaded
//! from an imported function. Guards path resolution that only differs when
//! code runs from a module outside the entry directory (see #247).

use std::path::{Path, PathBuf};
use std::process::Command;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn fixture_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi_file_app"),
        &app,
    );
    (dir, app)
}

fn free_port() -> String {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string()
}

#[test]
fn multi_file_app_renders_from_lib_routes_and_jobs() {
    let (_dir, app) = fixture_copy();
    let port = free_port();
    // Run from a different working directory: resolution must follow the
    // entry file, not the process cwd.
    let out = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .current_dir(std::env::temp_dir())
        .args([
            "test",
            app.join("main.tnt").to_str().unwrap(),
            "--port",
            &port,
            "--get",
            "/",
            "--get",
            "/compiled",
            "--get",
            "/items",
            "--get",
            "/about",
        ])
        .env("NTNT_ENV", "development")
        .env_remove("NTNT_TYPE_MODE")
        .output()
        .expect("run ntnt test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let all = format!("{stdout}{stderr}");

    assert!(all.contains("4 requests, 4 passed, 0 failed"), "{all}");
    // template() + partial, called from lib/
    assert!(all.contains("<p>Hello Ada</p>"), "{all}");
    // compile() + render() from lib/
    assert!(all.contains("<p>Hello Bob</p>"), "{all}");
    // string append loop in lib/ feeding a template
    assert!(
        all.contains("<li>item 0</li><li>item 1</li><li>item 2</li>"),
        "{all}"
    );
    // file route importing ../lib/ and rendering a template
    assert!(all.contains("<p>About this fixture</p>"), "{all}");
    // every page includes the partial from views/partials/
    assert_eq!(all.matches("<h1>Fixture Site</h1>").count(), 4, "{all}");
    assert!(!all.contains("Failed to load template"), "{all}");
}
