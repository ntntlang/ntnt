//! Run a CLI child with a deadline so a stalled process fails its test instead of
//! hanging the suite. Output goes to anonymous temporary files, so a full pipe never
//! blocks the child; at the deadline the child is killed and reaped.
#![allow(dead_code)]
use std::io::{Read, Seek};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

/// Generous for loaded CI runners; these commands normally finish in well under a second.
pub const DEADLINE: Duration = Duration::from_secs(60);

pub trait BoundedOutput {
    /// `Command::output` that fails the test when the child outlives [`DEADLINE`].
    fn bounded_output(&mut self) -> Output;
}
impl BoundedOutput for Command {
    fn bounded_output(&mut self) -> Output {
        output_within(self, DEADLINE).unwrap_or_else(|stalled| panic!("{stalled}"))
    }
}

/// A child that outlived its deadline: the status it was reaped with after the kill,
/// and a report of what it printed.
#[derive(Debug)]
pub struct Stalled {
    /// What `wait` returned after the kill; only a reaped child has a status.
    pub status: ExitStatus,
    pub report: String,
}
impl std::fmt::Display for Stalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.report)
    }
}

/// Kills and reaps the child however waiting ends, including a panic.
struct Reaped(Child);
impl Reaped {
    /// Kill the child unless it already exited, then reap it and return how it ended.
    fn stop(&mut self) -> std::io::Result<ExitStatus> {
        // A child that already exited is fine: wait reaps it either way.
        let _ = self.0.kill();
        self.0.wait()
    }
}
impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Run `command` to completion, or kill and reap it at `limit`. Either way the
/// status is the one the child was reaped with.
pub fn output_within(command: &mut Command, limit: Duration) -> Result<Output, Stalled> {
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
    let exited = loop {
        if child.0.try_wait().unwrap().is_some() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Reaping gives the status, after a kill if the child is still running.
    let status = child
        .stop()
        .unwrap_or_else(|error| panic!("cannot reap {command:?}: {error}"));
    let read = |file: &mut std::fs::File| {
        let mut bytes = Vec::new();
        file.rewind().unwrap();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    };
    let (stdout, stderr) = (read(&mut stdout), read(&mut stderr));
    if exited {
        return Ok(Output {
            status,
            stdout,
            stderr,
        });
    }
    Err(Stalled {
        status,
        report: format!(
            "{command:?} did not exit within {limit:?}; it was killed and reaped ({status})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        ),
    })
}
