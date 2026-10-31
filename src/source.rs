//! Provenance of file-backed interpreter input (DD-041 prerequisite).
//!
//! A SourceFile resolves once, reads that resolved path, and keeps the exact
//! bytes parsed. ParsedSource binds an immutable AST to those bytes; a path or
//! an independently constructed Program cannot attest to what was evaluated.
//! Snapshots describe observed interpreter inputs, not a complete/frozen program
//! generation, endpoint parity, execution success, or worker eligibility. Lazy
//! imports remain lazy; tasks do not inherit a parent's provenance.

use crate::ast::Program;
use crate::error::{IntentError, Result};
use crate::lexer::Lexer;
use crate::parser::Parser;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One resolved file and the exact UTF-8 input read from it. Not reread at parse
/// or snapshot time. Debug intentionally does not include application source.
///
/// ```
/// use ntnt::{interpreter::Interpreter, source::SourceFile};
/// let dir = tempfile::tempdir()?;
/// let path = dir.path().join("main.tnt");
/// std::fs::write(&path, "assert(1 == 1)")?;
/// let file = SourceFile::read(&path)?;
/// let parsed = file.parse()?;
/// let mut interpreter = Interpreter::new();
/// interpreter.eval_source(&parsed)?;
/// assert_eq!(interpreter.source_snapshot()?.files().len(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct SourceFile {
    path: PathBuf,
    text: String,
    digest: [u8; 32],
    identity_known: bool,
}

impl SourceFile {
    pub fn read(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        match path.canonicalize() {
            Ok(resolved) => Self::read_path(resolved, true),
            Err(_) => {
                // Legacy CLI/file loading can read pipes such as /dev/stdin
                // even when canonicalization cannot produce a real identity.
                // Keep execution working, but never attest to that fallback.
                let fallback = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::env::current_dir()?.join(path)
                };
                Self::read_path(fallback, false)
            }
        }
    }

    /// The import cache has already chosen its key. Validate that identity,
    /// but never replace it with a newly resolved target before reading.
    pub(crate) fn read_resolved(path: PathBuf) -> std::io::Result<Self> {
        let known = path.canonicalize().is_ok_and(|resolved| resolved == path);
        Self::read_path(path, known)
    }

    fn read_path(path: PathBuf, identity_known: bool) -> std::io::Result<Self> {
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let identity_known = identity_known && file.metadata().is_ok_and(|m| m.is_file());
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let digest = Sha256::digest(text.as_bytes()).into();
        Ok(Self {
            path,
            text,
            digest,
            identity_known,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn parse(&self) -> Result<ParsedSource<'_>> {
        let tokens = Lexer::new(&self.text).collect();
        let program = Parser::new(tokens)
            .with_resolved_source_file(self.path.clone())
            .parse()?;
        Ok(ParsedSource {
            file: self,
            program,
        })
    }
}

/// Immutable parser output tied to its actual input, without adding metadata to
/// the public Program AST or trusting an arbitrary AST-plus-file-name pairing.
pub struct ParsedSource<'a> {
    file: &'a SourceFile,
    program: Program,
}

impl ParsedSource<'_> {
    pub fn program(&self) -> &Program {
        &self.program
    }

    pub fn file(&self) -> &SourceFile {
        self.file
    }
}

/// A point-in-time source-only snapshot. Path encoding is native to the running
/// platform; this is not a portable release/build fingerprint. Each stored file
/// has one observed digest, and source-less Program/block evaluation makes a snapshot
/// unavailable rather than silently claiming full provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSnapshot {
    files: BTreeMap<PathBuf, [u8; 32]>,
    digest: [u8; 32],
}

impl SourceSnapshot {
    pub fn files(&self) -> &BTreeMap<PathBuf, [u8; 32]> {
        &self.files
    }

    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

#[derive(Default)]
pub(crate) struct SourceLedger {
    // Retain only the first digest plus one conflict bit, not unbounded history
    // across edits. A reload cannot silently relabel old cached definitions.
    files: BTreeMap<PathBuf, ([u8; 32], bool)>,
    file_entry: bool,
    source_less: Cell<bool>,
}

impl SourceLedger {
    pub(crate) fn file_entry(&mut self) {
        self.file_entry = true;
    }

    pub(crate) fn source_less(&self) {
        self.source_less.set(true);
    }

    pub(crate) fn record(&mut self, source: &ParsedSource<'_>) {
        let file = source.file;
        if !file.identity_known {
            self.source_less();
        }
        self.files
            .entry(file.path.clone())
            .and_modify(|(first, mixed)| *mixed |= *first != file.digest)
            .or_insert((file.digest, false));
    }

    pub(crate) fn snapshot(&self) -> Result<SourceSnapshot> {
        if !self.file_entry || self.source_less.get() {
            return Err(IntentError::runtime_error(
                "source provenance unavailable: execution includes an untracked/source-less entry",
            ));
        }
        let mut files = BTreeMap::new();
        for (path, (digest, mixed)) in &self.files {
            if *mixed {
                return Err(IntentError::runtime_error(format!(
                    "source provenance is mixed: different bytes evaluated for '{}'",
                    path.display()
                )));
            }
            files.insert(path.clone(), *digest);
        }
        let mut hash = Sha256::new();
        hash.update(b"ntnt-loaded-sources-v1");
        frame(&mut hash, std::env::consts::OS.as_bytes());
        for (path, digest) in &files {
            frame(&mut hash, path.as_os_str().as_encoded_bytes());
            frame(&mut hash, digest);
        }
        Ok(SourceSnapshot {
            files,
            digest: hash.finalize().into(),
        })
    }
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_order_independent_but_include_source_identity() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.tnt");
        let b = dir.path().join("b.tnt");
        std::fs::write(&a, "let n = 1").unwrap();
        std::fs::write(&b, "let n = 1").unwrap();
        let a = SourceFile::read(a).unwrap();
        let b = SourceFile::read(b).unwrap();
        let a = a.parse().unwrap();
        let b = b.parse().unwrap();
        let mut first = SourceLedger::default();
        first.file_entry();
        first.record(&a);
        let single = first.snapshot().unwrap();
        first.record(&b);
        let mut second = SourceLedger::default();
        second.file_entry();
        second.record(&b);
        assert_ne!(single.digest(), second.snapshot().unwrap().digest());
        second.record(&a);
        assert_eq!(first.snapshot().unwrap(), second.snapshot().unwrap());
    }

    #[test]
    fn frame_boundaries_cannot_alias() {
        let mut a = Sha256::new();
        frame(&mut a, b"ab");
        frame(&mut a, b"c");
        let mut b = Sha256::new();
        frame(&mut b, b"a");
        frame(&mut b, b"bc");
        assert_ne!(a.finalize(), b.finalize());
    }
}
