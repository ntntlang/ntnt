//! End-to-end compatibility across the retention ownership change.
use std::io::{Read, Seek};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct OwnedProcess(Child);
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_fixture(source_text: &str, marker: &str) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("compatibility.tnt");
    std::fs::write(&source, source_text).unwrap();
    let mut log = tempfile::tempfile().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command.arg("run").arg(&source).current_dir(dir.path());
    // Exercise zero-config defaults, not the calling operator's advanced tuning.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NTNT_TASK_") {
            command.env_remove(name);
        }
    }
    let mut child = OwnedProcess(
        command
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log.try_clone().unwrap()))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "task retention CLI timed out");
        std::thread::sleep(Duration::from_millis(10));
    };
    log.rewind().unwrap();
    let mut output = String::new();
    log.read_to_string(&mut output).unwrap();
    assert!(status.success(), "{output}");
    assert!(output.contains(marker), "{output}");
}

#[test]
fn consumed_history_preserves_public_task_results_and_structured_concurrency() {
    run_fixture(
        include_str!("fixtures/task_retention/compatibility.tnt"),
        "PASS: retained task handles",
    );
}

#[test]
fn consuming_a_task_releases_its_serialized_channel_sender() {
    run_fixture(
        include_str!("fixtures/task_retention/sender_disconnect.tnt"),
        "PASS: consuming a task releases its serialized channel sender",
    );
}
