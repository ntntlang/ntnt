//! Multi-file fixture app (#260): an entry file with `lib/` helpers, `views/`
//! templates and partials, file-based `routes/`, and a `jobs/` directory loaded
//! from an imported function. Guards path resolution that only differs when
//! code runs from a module outside the entry directory (see #247).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

/// Run `ntnt test` with a hard deadline so a stuck handler fails the test
/// instead of hanging CI. Output goes to files so a full pipe cannot block.
fn run_with_deadline(dir: &Path, app: &Path, paths: &[&str]) -> String {
    let port = free_port();
    let mut args = vec![
        "test".to_string(),
        app.join("main.tnt").to_string_lossy().to_string(),
        "--port".to_string(),
        port,
    ];
    for path in paths {
        args.push("--get".to_string());
        args.push(path.to_string());
    }
    let stdout_path = dir.join("stdout");
    let stderr_path = dir.join("stderr");
    // Run from a private working directory outside the app: resolution must
    // follow the entry file, not the process cwd, and parallel runs must not
    // share any files the child creates in its cwd.
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .current_dir(&cwd)
        .args(&args)
        .env("NTNT_ENV", "development")
        .env_remove("NTNT_TYPE_MODE")
        .stdout(Stdio::from(std::fs::File::create(&stdout_path).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&stderr_path).unwrap()))
        .spawn()
        .expect("spawn ntnt test");
    let deadline = Instant::now() + Duration::from_secs(60);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break true;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut output = String::new();
    std::fs::File::open(&stdout_path)
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let mut stderr = String::new();
    std::fs::File::open(&stderr_path)
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!timed_out, "ntnt test exceeded 60s: {output}{stderr}");
    // The fixture uses an in-memory job queue; no job store may be written.
    for place in [&cwd, app] {
        assert!(
            !place.join("jobs.db").exists(),
            "fixture wrote jobs.db in {}",
            place.display()
        );
    }
    output
}

/// Split `ntnt test` stdout into one block per request, keyed by path.
fn responses(stdout: &str, paths: &[&str]) -> Vec<String> {
    let blocks: Vec<&str> = stdout.split("[REQUEST ").skip(1).collect();
    assert_eq!(blocks.len(), paths.len(), "{stdout}");
    blocks
        .iter()
        .zip(paths)
        .map(|(block, path)| {
            let header = block.lines().next().unwrap_or("");
            assert!(
                header.ends_with(&format!("GET {path}")),
                "request order: expected {path}, got {header}"
            );
            block.to_string()
        })
        .collect()
}

#[test]
fn multi_file_app_renders_from_lib_routes_and_jobs() {
    let (dir, app) = fixture_copy();
    let paths = ["/", "/compiled", "/items", "/about", "/jobs"];
    let stdout = run_with_deadline(dir.path(), &app, &paths);
    assert!(
        stdout.contains("5 requests, 5 passed, 0 failed"),
        "{stdout}"
    );
    let blocks = responses(&stdout, &paths);

    // Each route must serve its own content: (path, expected, must not contain).
    let expectations: [(&str, &str, &[&str]); 5] = [
        // template() + partial, called from lib/
        (
            "/",
            "<p>Hello Ada</p>",
            &["Hello Bob", "About this fixture", "<li>"],
        ),
        // compile() + render() from lib/
        (
            "/compiled",
            "<p>Hello Bob</p>",
            &["Hello Ada", "About this fixture", "<li>"],
        ),
        // string append loop in lib/ feeding a template
        (
            "/items",
            "<ul><li>item 0</li><li>item 1</li><li>item 2</li></ul>",
            &["Hello", "About this fixture"],
        ),
        // file route importing ../lib/ and rendering a template
        ("/about", "<p>About this fixture</p>", &["Hello", "<li>"]),
        // jobs("jobs/") called from lib/setup.tnt registered the fixture job
        ("/jobs", "job registered", &["job missing"]),
    ];
    for ((path, expected, absent), block) in expectations.iter().zip(&blocks) {
        assert!(block.contains("[RESPONSE] 200"), "{path}: {block}");
        assert!(block.contains(expected), "{path}: {block}");
        for other in *absent {
            assert!(!block.contains(other), "{path} served {other}: {block}");
        }
        if *path != "/jobs" {
            // partial from views/partials/ on every page
            assert!(block.contains("<h1>Fixture Site</h1>"), "{path}: {block}");
        }
        assert!(
            !block.contains("Failed to load template"),
            "{path}: {block}"
        );
    }
}
