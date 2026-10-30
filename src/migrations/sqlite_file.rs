//! Native file binding without opening/closing extra Unix descriptors while
//! SQLite owns POSIX locks. Filesystem administrators remain trusted authority.
use super::inventory::json_digest;
use super::{Diagnostic, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) struct FileBinding {
    path: PathBuf,
    id: String,
    #[cfg(windows)]
    guard: std::fs::File,
}
fn bad() -> Diagnostic {
    Diagnostic::new("DB_FILE_IDENTITY", "SQLite requires an existing regular, single-link local file at an absolute, non-symlink path. File details suppressed.")
}
impl FileBinding {
    pub fn capture(path: &Path) -> Result<Self> {
        if !path.is_absolute() || path.to_str().is_none_or(|p| p.contains('\0')) {
            return Err(bad());
        }
        // Resolve directory aliases once; reject linked namespace components.
        for parent in path.ancestors() {
            if std::fs::symlink_metadata(parent)
                .map_err(|_| bad())?
                .file_type()
                .is_symlink()
            {
                return Err(bad());
            }
        }
        let path = path.canonicalize().map_err(|_| bad())?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| bad())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(bad());
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
                .map_err(|_| bad())?
        };
        let mut binding = Self {
            path,
            id: String::new(),
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
    pub fn check(&self) -> Result<Value> {
        let (id, authority) = self.observe()?;
        if id != self.id {
            return Err(bad());
        }
        Ok(authority)
    }
    fn observe(&self) -> Result<(String, Value)> {
        let md = std::fs::symlink_metadata(&self.path).map_err(|_| bad())?;
        if !md.is_file() || md.file_type().is_symlink() {
            return Err(bad());
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
                return Err(bad());
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
                return Err(bad());
            }
            // SAFETY: successful API call initialized the output.
            let info = unsafe { info.assume_init() };
            if info.nNumberOfLinks != 1 {
                return Err(bad());
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
            Err(bad())
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
                return Err(bad());
            }
        }
        #[cfg(not(unix))]
        let _ = conn;
        self.check()
    }
}
