//! Capture a regular input once; reject links and nonregular replacement races.
use std::fs::{File, Metadata, OpenOptions};
use std::path::Path;

pub(super) fn open(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // NONBLOCK prevents an attacker replacing the regular file with a FIFO
        // between lstat and open from hanging capture before its deadline.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        // Do not follow a replacement reparse point; deny writes/deletion while
        // the captured handle is open, without relying on unstable Metadata APIs.
        options
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}
#[cfg(unix)]
pub(super) fn single_link(_file: &File, metadata: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() == 1
}
#[cfg(windows)]
pub(super) fn single_link(file: &File, _metadata: &Metadata) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: a valid live file handle and writable correctly sized output.
    let success = unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) };
    if success == 0 {
        return false;
    }
    // SAFETY: the successful API call initialized the structure.
    unsafe { info.assume_init().nNumberOfLinks == 1 }
}
#[cfg(not(any(unix, windows)))]
pub(super) fn single_link(_file: &File, _metadata: &Metadata) -> bool {
    false
}

pub(super) fn same(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (a.dev(), a.ino(), a.ctime(), a.ctime_nsec())
            != (b.dev(), b.ino(), b.ctime(), b.ctime_nsec())
        {
            return false;
        }
    }
    a.is_file()
        && b.is_file()
        && !a.file_type().is_symlink()
        && !b.file_type().is_symlink()
        && a.len() == b.len()
        && a.modified().ok() == b.modified().ok()
}
