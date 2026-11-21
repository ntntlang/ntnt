//! Which file a runtime error's line belongs to, recorded while it unwinds
//! (#412).
//!
//! An error takes its line from the innermost `Located` statement it leaves,
//! which is in the frame the error was raised in. The first boundary the error
//! crosses after that is the end of that frame, and names the file of the
//! line: the file whose module scope a called function was defined in, or the
//! file being evaluated. The next statement the error leaves is the call site
//! in the enclosing frame, the boundary after that names its file, and so on
//! outward.
//!
//! Only error paths reach this: statements and calls that succeed do no extra
//! work. Nothing here is read for evaluation, imports, templates or any other
//! path resolution; those keep using `current_file`, which this never changes.

use crate::error::{ErrorOrigin, IntentError, OriginFrame, OriginStep};
use std::path::PathBuf;
use std::rc::Rc;

/// The file a module scope was evaluated from. Stored once on the scope when a
/// file loader creates it, so it lives exactly as long as the functions that
/// were defined there (their closures keep the scope alive).
pub(crate) type SourceOrigin = Rc<PathBuf>;

#[derive(Debug, Clone)]
enum FrameFile {
    /// Named when the error leaves the frame this line is in.
    Pending,
    /// The frame's scope has no recorded file: the entry file's own scope, or
    /// a scope an embedder or a task built.
    Unrecorded,
    Known(SourceOrigin),
}

#[derive(Debug, Clone)]
struct Frame {
    file: FrameFile,
    line: usize,
    step: Option<OriginStep>,
}

/// Provenance of the error that is unwinding, keyed by its code and line. A
/// newer error replaces it as soon as it is located, so the trace of an error
/// that was caught and dropped is never reported for another one.
#[derive(Debug, Default)]
pub(crate) struct ErrorTrace {
    error: Option<(&'static str, usize)>,
    /// One per frame the error has left, so never more than the call depth.
    frames: Vec<Frame>,
    /// Set when the error left a frame: the next statement it leaves is the
    /// call site, in the enclosing frame, that entered it this way.
    awaiting: Option<OriginStep>,
}

impl ErrorTrace {
    /// A statement has just given `error` its line: a new error starts here.
    pub(crate) fn located(&mut self, error: &IntentError) {
        match error.line() {
            Some(line) => self.start(error.error_code(), line),
            None => self.clear(),
        }
    }

    /// `error`, already located, left the statement that starts on `line`.
    pub(crate) fn left_statement(&mut self, error: &IntentError, line: usize) {
        if !self.describes(error) {
            return;
        }
        if let Some(step) = self.awaiting.take() {
            self.frames.push(Frame {
                file: FrameFile::Pending,
                line,
                step: Some(step),
            });
        }
    }

    /// `error` left a function call (`Call`) or the evaluation of a file
    /// (`Import`) whose code comes from `file`; `None` when the scope has no
    /// recorded file.
    pub(crate) fn left_frame(
        &mut self,
        error: &IntentError,
        step: OriginStep,
        file: Option<SourceOrigin>,
    ) {
        let Some(line) = error.line() else {
            // Not located yet: a statement in the enclosing frame gives it
            // its line, and that frame's boundary its file.
            return;
        };
        if !self.describes(error) {
            // Raised with its line already set (a contract clause, a parse
            // error) and not traced yet.
            self.start(error.error_code(), line);
        }
        if let Some(frame) = self.frames.last_mut() {
            if matches!(frame.file, FrameFile::Pending) {
                frame.file = file.map_or(FrameFile::Unrecorded, FrameFile::Known);
            }
        }
        self.awaiting = Some(step);
    }

    /// Forget the current trace. Called where an error is created with its
    /// line already set, so that an older, caught error with the same code and
    /// line can never be taken for it.
    pub(crate) fn clear(&mut self) {
        self.error = None;
        self.frames.clear();
        self.awaiting = None;
    }

    /// The recorded locations of `error`, or `None` if the trace is not
    /// about it.
    pub(crate) fn origin(&self, error: &IntentError) -> Option<ErrorOrigin> {
        if !self.describes(error) {
            return None;
        }
        let frames = self
            .frames
            .iter()
            .map(|frame| OriginFrame {
                file: match &frame.file {
                    FrameFile::Known(path) => Some(path.as_ref().clone()),
                    FrameFile::Pending | FrameFile::Unrecorded => None,
                },
                line: frame.line,
                step: frame.step,
            })
            .collect();
        Some(ErrorOrigin { frames })
    }

    fn describes(&self, error: &IntentError) -> bool {
        self.error
            .is_some_and(|(code, line)| code == error.error_code() && error.line() == Some(line))
    }

    fn start(&mut self, code: &'static str, line: usize) {
        self.clear();
        self.error = Some((code, line));
        self.frames.push(Frame {
            file: FrameFile::Pending,
            line,
            step: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(line: usize) -> IntentError {
        IntentError::runtime_error("boom").at_line(line)
    }

    fn file(name: &str) -> Option<SourceOrigin> {
        Some(Rc::new(PathBuf::from(name)))
    }

    fn summary(origin: &ErrorOrigin) -> Vec<(Option<String>, usize, Option<OriginStep>)> {
        origin
            .frames
            .iter()
            .map(|f| {
                (
                    f.file.as_ref().map(|p| p.display().to_string()),
                    f.line,
                    f.step,
                )
            })
            .collect()
    }

    #[test]
    fn each_boundary_names_the_file_of_the_line_inside_it() {
        let mut trace = ErrorTrace::default();
        let error = runtime(9);
        trace.located(&error);
        // An enclosing statement in the same frame is not a call site.
        trace.left_statement(&error, 8);
        trace.left_frame(&error, OriginStep::Call, file("lib/b.tnt"));
        trace.left_statement(&error, 5);
        trace.left_statement(&error, 4);
        trace.left_frame(&error, OriginStep::Call, None);
        trace.left_statement(&error, 3);
        trace.left_frame(&error, OriginStep::Import, file("main.tnt"));
        let origin = trace.origin(&error).unwrap();
        assert_eq!(
            summary(&origin),
            vec![
                (Some("lib/b.tnt".into()), 9, None),
                (None, 5, Some(OriginStep::Call)),
                (Some("main.tnt".into()), 3, Some(OriginStep::Call)),
            ]
        );
    }

    #[test]
    fn a_new_error_replaces_the_trace_of_a_caught_one_with_the_same_line() {
        let mut trace = ErrorTrace::default();
        let caught = runtime(9);
        trace.located(&caught);
        trace.left_frame(&caught, OriginStep::Call, file("lib/checks.tnt"));
        // `caught` is dropped by `otherwise`; main raises its own E005 on 9.
        let error = runtime(9);
        trace.located(&error);
        trace.left_frame(&error, OriginStep::Import, file("main.tnt"));
        let origin = trace.origin(&error).unwrap();
        assert_eq!(summary(&origin), vec![(Some("main.tnt".into()), 9, None)]);
    }

    #[test]
    fn an_error_raised_with_its_line_starts_at_the_first_boundary() {
        let mut trace = ErrorTrace::default();
        // A violation in lib/math.tnt on line 4, caught by the caller.
        let caught = IntentError::contract_violation("Precondition failed").at_line(4);
        trace.left_frame(&caught, OriginStep::Call, file("lib/math.tnt"));
        // A second violation, also on line 4, from a function in the entry
        // file's own scope. It is created with its line, so it clears first.
        trace.clear();
        let violation = IntentError::contract_violation("Precondition failed").at_line(4);
        trace.left_frame(&violation, OriginStep::Call, None);
        trace.left_statement(&violation, 13);
        trace.left_frame(&violation, OriginStep::Import, file("main.tnt"));
        let origin = trace.origin(&violation).unwrap();
        assert_eq!(
            summary(&origin),
            vec![
                (None, 4, None),
                (Some("main.tnt".into()), 13, Some(OriginStep::Call)),
            ]
        );
        // Another code or line, or no line at all, has no recorded origin.
        assert!(trace.origin(&runtime(4)).is_none());
        assert!(trace
            .origin(&IntentError::runtime_error("unlocated"))
            .is_none());
    }
}
