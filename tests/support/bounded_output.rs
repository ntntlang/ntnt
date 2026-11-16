//! Run a CLI child with a deadline so a stalled process fails its test instead of
//! hanging the suite. Output goes to anonymous temporary files, so a full pipe never
//! blocks the child; at the deadline the child is killed and reaped.
#![allow(dead_code)]
use std::io::{Read, Seek};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Generous for loaded CI runners; these commands normally finish in well under a second.
pub const DEADLINE: Duration = Duration::from_secs(60);

pub trait BoundedOutput {
    /// `Command::output` that fails the test when the child outlives [`DEADLINE`].
    fn bounded_output(&mut self) -> Output;
}
impl BoundedOutput for Command {
    fn bounded_output(&mut self) -> Output {
        output_within(self, DEADLINE).unwrap_or_else(|failure| panic!("{failure}"))
    }
}

/// Kills and reaps the child however waiting ends, including a panic.
struct Reaped(Child);
impl Drop for Reaped {
    fn drop(&mut self) {
        // A child that already exited is fine: wait reaps it either way.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Run `command` to completion, or kill and reap it at `limit` and return what it printed.
pub fn output_within(command: &mut Command, limit: Duration) -> Result<Output, String> {
    let capture = || tempfile::tempfile().expect("temporary output file");
    let (mut stdout, mut stderr) = (capture(), capture());
    let mut child = Reaped(
        command
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().unwrap())
            .stderr(stderr.try_clone().unwrap())
            .spawn()
            .unwrap_or_else(|error| panic!("cannot start {command:?}: {error}")),
    );
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(child);
    let read = |file: &mut std::fs::File| {
        let mut bytes = Vec::new();
        file.rewind().unwrap();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    };
    let (stdout, stderr) = (read(&mut stdout), read(&mut stderr));
    match status {
        Some(status) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        None => Err(format!(
            "{command:?} did not exit within {limit:?}; it was killed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        )),
    }
}
