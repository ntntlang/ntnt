//! Native file binding without opening/closing extra Unix descriptors while
//! SQLite owns POSIX locks. Filesystem administrators remain trusted authority.
use super::inventory::json_digest;
use super::{Diagnostic, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) struct FileBinding {
    path: PathBuf,
    id: String,
    owner: Owner,
    #[cfg(windows)]
    guard: std::fs::File,
}
/// Whose file a binding is, for its diagnostics: the role (`target` or
/// `scratch`) and the connection secret that names the file. A refusal names
/// these and the rule the path broke, never the path (#422).
pub(super) struct Owner {
    pub role: &'static str,
    pub secret: String,
}
/// The rule a SQLite file path broke.
enum Rule {
    NotText,
    Relative,
    Stream,
    Missing,
    Inaccessible(std::io::Error),
    Symlink,
    NotRegular,
    Linked,
    Replaced,
    Moved,
    #[cfg(not(any(unix, windows)))]
    Unsupported,
}
impl Owner {
    fn refuse(&self, rule: Rule) -> Diagnostic {
        let secret = &self.secret;
        let (role, example) = match self.role {
            "scratch" => ("Scratch", "data/replay.sqlite"),
            _ => ("Target", "data/app.sqlite"),
        };
        let (problem, next_action) = match rule {
            Rule::NotText => (
                "names a path that is not UTF-8 text or holds a NUL byte".to_owned(),
                format!("Set {secret} to the database file's absolute path as UTF-8 text."),
            ),
            Rule::Relative => (
                "names a relative path; SQLite files are named by absolute path".to_owned(),
                format!("Set {secret} to the file's absolute path, for example \"$(pwd -P)/{example}\"; `ntnt db` never resolves a relative one."),
            ),
            Rule::Stream => (
                "names a path with `:` in a file name (NTFS alternate streams are not supported)".to_owned(),
                format!("Set {secret} to a path whose file names hold no `:`; a Windows drive prefix is fine."),
            ),
            Rule::Missing => (
                "names a file that does not exist".to_owned(),
                match self.role {
                    "scratch" => format!("Create a separate, empty scratch file first, for example `touch {example}` (an empty file is a valid SQLite database), or set {secret} to the absolute path of an existing one; `ntnt db` never creates database files."),
                    _ => format!("Create the file first, for example `touch {example}` (an empty file is a valid SQLite database), or set {secret} to the absolute path of the existing file; `ntnt db` never creates database files."),
                },
            ),
            Rule::Inaccessible(error) => (
                format!("names a path that cannot be inspected ({error})"),
                "Make the file and every directory above it accessible to this user, then rerun.".to_owned(),
            ),
            Rule::Symlink => (
                "names a path through a symbolic link".to_owned(),
                format!("Set {secret} to the file's physical path, with no symbolic link in it: `pwd -P` prints the physical directory, as in \"$(pwd -P)/{example}\" (on macOS, /private/var rather than /var)."),
            ),
            Rule::NotRegular => (
                "names something that is not a regular file".to_owned(),
                format!("Set {secret} to the absolute path of a regular SQLite file, not a directory or special file."),
            ),
            Rule::Linked => (
                "names a file with more than one hard link".to_owned(),
                "Give the file a single link: remove its other hard links, or use a copy (a copy is a new file, so plan again).".to_owned(),
            ),
            Rule::Replaced => (
                "names a file that was replaced while `ntnt db` was using it".to_owned(),
                "Rerun once nothing replaces the database file; a different file needs a new plan.".to_owned(),
            ),
            Rule::Moved => (
                "names a file that was moved or deleted while `ntnt db` had it open".to_owned(),
                "Rerun once nothing moves or deletes the database file; a different file needs a new plan.".to_owned(),
            ),
            #[cfg(not(any(unix, windows)))]
            Rule::Unsupported => (
                "names a file this platform cannot bind".to_owned(),
                "Run `ntnt db` on Unix or Windows.".to_owned(),
            ),
        };
        let mut d = Diagnostic::new(
            "DB_FILE_IDENTITY",
            &format!("{role} connection secret {secret} {problem}. The path is not shown."),
        );
        d.next_action = next_action;
        d
    }
    /// A path that does not exist, or the error that kept it from being inspected.
    fn missing_or(&self, error: std::io::Error) -> Diagnostic {
        match error.kind() {
            std::io::ErrorKind::NotFound => self.refuse(Rule::Missing),
            _ => self.refuse(Rule::Inaccessible(error)),
        }
    }
}
impl FileBinding {
    pub fn capture(path: &Path, owner: Owner) -> Result<Self> {
        if path.to_str().is_none_or(|p| p.contains('\0')) {
            return Err(owner.refuse(Rule::NotText));
        }
        if !path.is_absolute() {
            return Err(owner.refuse(Rule::Relative));
        }
        // NTFS streams share the file index/timestamps but have independent
        // contents. Reject stream syntax in filename components portably;
        // a Windows drive-prefix colon is not a Normal component.
        if path.components().any(|part| {
            matches!(part,
            std::path::Component::Normal(name) if name.to_string_lossy().contains(':'))
        }) {
            return Err(owner.refuse(Rule::Stream));
        }
        // Resolve directory aliases once; reject linked namespace components.
        for parent in path.ancestors() {
            if std::fs::symlink_metadata(parent)
                .map_err(|error| owner.missing_or(error))?
                .file_type()
                .is_symlink()
            {
                return Err(owner.refuse(Rule::Symlink));
            }
        }
        let path = path
            .canonicalize()
            .map_err(|error| owner.missing_or(error))?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| owner.missing_or(error))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(owner.refuse(Rule::NotRegular));
        }
        #[cfg(windows)]
        let guard = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
            };
            // Allow SQLite access but prevent deletion/replacement while connected.
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(&path)
                .map_err(|error| owner.missing_or(error))?
        };
        let mut binding = Self {
            path,
            id: String::new(),
            owner,
            #[cfg(windows)]
            guard,
        };
        binding.id = binding.observe()?.0;
        Ok(binding)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn owner(&self) -> &Owner {
        &self.owner
    }
    pub fn check(&self) -> Result<Value> {
        let (id, authority) = self.observe()?;
        if id != self.id {
            return Err(self.owner.refuse(Rule::Replaced));
        }
        Ok(authority)
    }
    fn observe(&self) -> Result<(String, Value)> {
        let md = std::fs::symlink_metadata(&self.path).map_err(|error| match error.kind() {
            // The file was there when the binding was made.
            std::io::ErrorKind::NotFound => self.owner.refuse(Rule::Moved),
            _ => self.owner.refuse(Rule::Inaccessible(error)),
        })?;
        if !md.is_file() || md.file_type().is_symlink() {
            return Err(self.owner.refuse(Rule::NotRegular));
        }
        let created = md
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos().to_string());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if md.nlink() != 1 {
                return Err(self.owner.refuse(Rule::Linked));
            }
            Ok((
                json_digest(&json!({"device":md.dev(),"inode":md.ino(),"created":created})),
                json!({"uid":md.uid(),"gid":md.gid(),"mode":md.mode()}),
            ))
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            };
            let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
            // SAFETY: retained live regular-file handle and correctly sized output.
            if unsafe { GetFileInformationByHandle(self.guard.as_raw_handle(), info.as_mut_ptr()) }
                == 0
            {
                return Err(self
                    .owner
                    .refuse(Rule::Inaccessible(std::io::Error::last_os_error())));
            }
            // SAFETY: successful API call initialized the output.
            let info = unsafe { info.assume_init() };
            if info.nNumberOfLinks != 1 {
                return Err(self.owner.refuse(Rule::Linked));
            }
            Ok((
                json_digest(
                    &json!({"volume":info.dwVolumeSerialNumber,"index_high":info.nFileIndexHigh,"index_low":info.nFileIndexLow,"created":created}),
                ),
                json!({"readonly":md.permissions().readonly(),"filesystem_acl":"externally_managed"}),
            ))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = created;
            Err(self.owner.refuse(Rule::Unsupported))
        }
    }
    /// SQLite's own VFS checks the *open database*, not just a pathname stat.
    /// Never open a second Unix file descriptor here: closing it could release
    /// SQLite's process-scoped POSIX locks on the same inode.
    pub fn check_connection(&self, conn: &rusqlite::Connection) -> Result<Value> {
        #[cfg(unix)]
        {
            let mut moved = 0i32;
            // SAFETY: a live SQLite connection, static NUL-terminated schema and
            // the integer output required by SQLITE_FCNTL_HAS_MOVED.
            let result = unsafe {
                rusqlite::ffi::sqlite3_file_control(
                    conn.handle(),
                    c"main".as_ptr(),
                    rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                    (&mut moved as *mut i32).cast(),
                )
            };
            if result != rusqlite::ffi::SQLITE_OK || moved != 0 {
                return Err(self.owner.refuse(Rule::Moved));
            }
        }
        #[cfg(not(unix))]
        let _ = conn;
        self.check()
    }
}
