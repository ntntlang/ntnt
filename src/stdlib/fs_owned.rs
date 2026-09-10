//! Owned temporary paths and same-directory atomic publication.
use crate::interpreter::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

type R<T> = Result<T, String>;
fn text(value: &Value) -> R<&str> {
    match value {
        Value::String(s) => Ok(s),
        _ => Err("invalid_argument: expected String".into()),
    }
}
fn options(value: Option<&Value>) -> R<Option<&HashMap<String, Value>>> {
    match value {
        None => Ok(None),
        Some(Value::Map(m)) => Ok(Some(m)),
        _ => Err("invalid_argument: options must be Map".into()),
    }
}
fn unpublished(error: impl std::fmt::Display, temp: tempfile::NamedTempFile) -> String {
    let mut message = format!("unpublished: {error}");
    if let Err(cleanup) = temp.close() {
        message.push_str(&format!(
            "; cleanup_failed: temporary resource may remain: {cleanup}"
        ));
    }
    message
}
pub(super) fn write_atomic(args: &[Value]) -> R<Value> {
    let destination = Path::new(text(&args[0])?);
    // Complete validation precedes any filesystem mutation.
    let content = match &args[1] {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Array(a) => a
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                _ => Err("invalid_argument: expected integer bytes in 0..255".to_string()),
            })
            .collect::<R<Vec<_>>>()?,
        _ => return Err("invalid_argument: content must be String or Array<Int>".into()),
    };
    let mut sync = true;
    let mut mode = None;
    if let Some(options) = options(args.get(2))? {
        for (key, value) in options {
            match (key.as_str(), value) {
                ("sync", Value::Bool(b)) => sync = *b,
                ("mode", Value::Int(n)) if (0..=511).contains(n) => mode = Some(*n as u32),
                _ => {
                    return Err(
                        "invalid_argument: expected sync Bool and mode integer 0..511".into(),
                    )
                }
            }
        }
    }
    #[cfg(not(unix))]
    if sync || mode.is_some() {
        return Err(
            "unsupported: use sync:false without POSIX mode on this platform; unpublished".into(),
        );
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_handle = if sync {
        Some(fs::File::open(parent).map_err(|e| format!("unpublished: open sync parent: {e}"))?)
    } else {
        None
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(".ntnt-atomic-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(mode.unwrap_or(0o600)));
    }
    let mut temp = builder
        .tempfile_in(parent)
        .map_err(|e| format!("unpublished: {e}"))?;
    if let Err(e) = temp.write_all(&content) {
        return Err(unpublished(e, temp));
    }
    if sync {
        if let Err(e) = temp.as_file().sync_all() {
            return Err(unpublished(e, temp));
        }
    }
    match temp.persist(destination) {
        Ok(file) => drop(file),
        Err(e) => return Err(unpublished(e.error, e.file)),
    }
    if let Some(parent) = parent_handle {
        parent
            .sync_all()
            .map_err(|e| format!("published: durability_uncertain: parent sync failed: {e}"))?;
    }
    Ok(Value::Unit)
}

static LIVE: AtomicUsize = AtomicUsize::new(0);
static OWNERS: OnceLock<Mutex<Vec<Weak<TempOwner>>>> = OnceLock::new();
struct TempPermit;
impl TempPermit {
    fn reserve() -> R<Self> {
        LIVE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 128).then_some(n + 1)
        })
        .map_err(|_| "capacity: at most 128 live temporary resources".to_string())?;
        Ok(Self)
    }
}
impl Drop for TempPermit {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
enum Resource {
    File(tempfile::NamedTempFile),
    Dir(tempfile::TempDir),
}
impl Resource {
    fn path(&self) -> &Path {
        match self {
            Self::File(f) => f.path(),
            Self::Dir(d) => d.path(),
        }
    }
    fn close(self) -> std::io::Result<()> {
        match self {
            Self::File(f) => f.close(),
            Self::Dir(d) => d.close(),
        }
    }
}
enum TempState {
    Open(Resource, TempPermit),
    Closed(R<()>),
}
pub struct TempOwner {
    state: Mutex<TempState>,
}
impl std::fmt::Debug for TempOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<opaque temporary resource>")
    }
}
fn unregister(owner: &TempOwner) -> R<()> {
    if let Some(registry) = OWNERS.get() {
        registry
            .lock()
            .map_err(|_| "system: poisoned temporary registry".to_string())?
            .retain(|w| !std::ptr::eq(w.as_ptr(), owner));
    }
    Ok(())
}
impl TempOwner {
    fn close(&self) -> R<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "system: poisoned temporary owner".to_string())?;
        if let TempState::Closed(result) = &*state {
            return result.clone();
        }
        let TempState::Open(resource, permit) =
            std::mem::replace(&mut *state, TempState::Closed(Ok(())))
        else {
            unreachable!()
        };
        let result = resource.close().map_err(|e| {
            format!("cleanup_failed: temporary resource may remain; no automatic retry: {e}")
        });
        drop(permit);
        *state = TempState::Closed(result.clone());
        drop(state);
        unregister(self)?;
        result
    }
}
impl Drop for TempOwner {
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            let _ = writeln!(std::io::stderr(), "temporary cleanup: {e}");
        }
    }
}
/// Close resources still captured by interpreter closure cycles. Snapshot before locking owners.
pub fn shutdown() -> R<()> {
    let owners = OWNERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "system: poisoned temporary registry".to_string())?
        .iter()
        .filter_map(Weak::upgrade)
        .collect::<Vec<_>>();
    let mut errors = Vec::new();
    for owner in owners {
        if let Err(e) = owner.close() {
            errors.push(e);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
pub(super) fn create_temp(args: &[Value], directory: bool) -> R<Value> {
    let mut parent = None;
    let mut prefix = "ntnt-";
    if let Some(options) = options(args.first())? {
        for (key, value) in options {
            match key.as_str() {
                "parent" => parent = Some(Path::new(text(value)?)),
                "prefix" => {
                    prefix = text(value)?;
                    if prefix.contains(['/', '\\', '\0']) {
                        return Err(
                            "invalid_argument: prefix cannot contain separators or NUL".into()
                        );
                    }
                }
                _ => {
                    return Err(
                        "invalid_argument: only parent and prefix options are accepted".into(),
                    )
                }
            }
        }
    }
    let permit = TempPermit::reserve()?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(if directory {
            0o700
        } else {
            0o600
        }));
    }
    let resource = if directory {
        Resource::Dir(
            match parent {
                Some(p) => builder.tempdir_in(p),
                None => builder.tempdir(),
            }
            .map_err(|e| format!("io: {e}"))?,
        )
    } else {
        Resource::File(
            match parent {
                Some(p) => builder.tempfile_in(p),
                None => builder.tempfile(),
            }
            .map_err(|e| format!("io: {e}"))?,
        )
    };
    let owner = Arc::new(TempOwner {
        state: Mutex::new(TempState::Open(resource, permit)),
    });
    // On registry failure drop the guard before owner Drop attempts cleanup.
    {
        let registry = OWNERS.get_or_init(|| Mutex::new(Vec::new())).lock();
        let mut registry =
            registry.map_err(|_| "system: poisoned temporary registry".to_string())?;
        registry.retain(|w| w.strong_count() != 0);
        registry.push(Arc::downgrade(&owner));
    }
    Ok(if directory {
        Value::TempDir(owner)
    } else {
        Value::TempFile(owner)
    })
}
fn temp_owner(value: &Value) -> R<&Arc<TempOwner>> {
    match value {
        Value::TempFile(o) | Value::TempDir(o) => Ok(o),
        _ => Err("invalid_argument: expected owned TempFile or TempDir".into()),
    }
}
pub(super) fn temp_path(args: &[Value]) -> R<Value> {
    let state = temp_owner(&args[0])?
        .state
        .lock()
        .map_err(|_| "system: poisoned temporary owner".to_string())?;
    match &*state {
        TempState::Open(resource, _) => Ok(Value::String(
            resource
                .path()
                .to_str()
                .ok_or("invalid_path: non-UTF8 temporary path")?
                .to_string(),
        )),
        TempState::Closed(_) => Err("closed: temporary resource".into()),
    }
}
pub(super) fn temp_close(args: &[Value]) -> R<Value> {
    temp_owner(&args[0])?.close()?;
    Ok(Value::Unit)
}

pub(super) fn lstat(args: &[Value]) -> R<Value> {
    let meta = fs::symlink_metadata(text(&args[0])?).map_err(|e| format!("io: {e}"))?;
    let timestamp = |time: std::io::Result<std::time::SystemTime>| {
        time.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0)
    };
    Ok(Value::Map(HashMap::from([
        (
            "size".into(),
            Value::Int(meta.len().min(i64::MAX as u64) as i64),
        ),
        ("is_file".into(), Value::Bool(meta.is_file())),
        ("is_dir".into(), Value::Bool(meta.is_dir())),
        ("is_symlink".into(), Value::Bool(meta.is_symlink())),
        ("modified".into(), Value::Int(timestamp(meta.modified()))),
        ("created".into(), Value::Int(timestamp(meta.created()))),
    ])))
}
pub(super) fn read_link(args: &[Value]) -> R<Value> {
    let target = fs::read_link(text(&args[0])?).map_err(|e| format!("io: {e}"))?;
    Ok(Value::String(
        target
            .to_str()
            .ok_or("invalid_path: non-UTF8 symlink target")?
            .to_string(),
    ))
}
pub(super) fn symlink(args: &[Value]) -> R<Value> {
    let target = text(&args[0])?;
    let path = text(&args[1])?;
    let kind = args.get(2).map(text).transpose()?.unwrap_or("file");
    if !matches!(kind, "file" | "dir") {
        return Err("invalid_argument: symlink kind must be file or dir".into());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, path).map_err(|e| format!("io: {e}"))?;
    #[cfg(windows)]
    if kind == "dir" {
        std::os::windows::fs::symlink_dir(target, path)
    } else {
        std::os::windows::fs::symlink_file(target, path)
    }
    .map_err(|e| format!("io: {e}"))?;
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, path);
        return Err("unsupported: symlink creation on this platform".into());
    }
    #[cfg(any(unix, windows))]
    Ok(Value::Unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temp_limit_creation_failure_terminal_cleanup_and_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let options = Value::Map(HashMap::from([(
            "parent".into(),
            Value::String(root.path().to_str().unwrap().into()),
        )]));
        let invalid = Value::Map(HashMap::from([(
            "parent".into(),
            Value::String(root.path().join("absent").to_str().unwrap().into()),
        )]));
        assert!(create_temp(&[invalid], false).is_err());
        assert_eq!(LIVE.load(Ordering::Acquire), 0);
        let resources = (0..128)
            .map(|_| create_temp(&[options.clone()], false).unwrap())
            .collect::<Vec<_>>();
        assert!(create_temp(&[options.clone()], true)
            .unwrap_err()
            .starts_with("capacity:"));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 128);
        drop(resources);
        assert_eq!(LIVE.load(Ordering::Acquire), 0);
        let resource = create_temp(&[options], false).unwrap();
        let Value::String(path) = temp_path(&[resource.clone()]).unwrap() else {
            panic!()
        };
        // Deliberately violate trusted-path ownership to force a portable consuming close error.
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let first = temp_close(&[resource.clone()]).unwrap_err();
        let second = temp_close(&[resource.clone()]).unwrap_err();
        assert!(first.contains("may remain"));
        assert_eq!(first, second);
        assert!(temp_path(&[resource]).is_err());
        fs::remove_dir(path).unwrap();
        let resource = create_temp(&[], true).unwrap();
        shutdown().unwrap();
        assert!(temp_path(&[resource]).is_err());
        assert_eq!(LIVE.load(Ordering::Acquire), 0);
        assert!(OWNERS.get().unwrap().lock().unwrap().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let parent = root.path().join(std::ffi::OsString::from_vec(vec![255]));
            fs::create_dir(&parent).unwrap();
            let permit = TempPermit::reserve().unwrap();
            let owner = Arc::new(TempOwner {
                state: Mutex::new(TempState::Open(
                    Resource::File(tempfile::NamedTempFile::new_in(&parent).unwrap()),
                    permit,
                )),
            });
            let value = Value::TempFile(owner);
            assert!(temp_path(&[value.clone()])
                .unwrap_err()
                .contains("non-UTF8"));
            temp_close(&[value]).unwrap();
        }
    }
}
