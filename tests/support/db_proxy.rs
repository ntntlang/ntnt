//! Test-only PostgreSQL wire fault fixture; never linked into the executable.
#![allow(dead_code)]
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;

enum Upstream {
    Unix(UnixStream),
    Tcp(std::net::TcpStream),
}
impl Upstream {
    fn connect(host: &str) -> std::io::Result<Self> {
        if std::path::Path::new(host).is_absolute() {
            UnixStream::connect(std::path::Path::new(host).join(".s.PGSQL.5432")).map(Self::Unix)
        } else {
            let stream = std::net::TcpStream::connect((host, 5432))?;
            stream.set_nodelay(true)?;
            Ok(Self::Tcp(stream))
        }
    }
    fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Self::Unix(s) => s.try_clone().map(Self::Unix),
            Self::Tcp(s) => s.try_clone().map(Self::Tcp),
        }
    }
    fn shutdown(&self, how: Shutdown) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.shutdown(how),
            Self::Tcp(s) => s.shutdown(how),
        }
    }
}
impl Read for Upstream {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.read(b),
            Self::Tcp(s) => s.read(b),
        }
    }
}
impl Write for Upstream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.write(b),
            Self::Tcp(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Fault {
    BeforeCommit,
    AfterCommit,
    AfterCommitUnavailable,
    AfterCreateIndex,
    Rollback,
}

pub struct Proxy {
    directory: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<UnixStream>>>,
    thread: Option<JoinHandle<()>>,
    pub fired: Arc<AtomicBool>,
}

fn frame(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut prefix = [0u8; 5];
    stream.read_exact(&mut prefix)?;
    let size = u32::from_be_bytes(prefix[1..5].try_into().unwrap()) as usize;
    if !(4..=16_777_216).contains(&size) {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut bytes = prefix.to_vec();
    bytes.resize(size + 1, 0);
    stream.read_exact(&mut bytes[5..])?;
    Ok(bytes)
}

impl Proxy {
    pub fn new(fault: Fault, commit: usize) -> Self {
        Self::for_host(fault, commit, std::env::var("NTNT_DB_TEST_HOST").unwrap())
    }

    fn for_host(fault: Fault, commit: usize, host: String) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join(".s.PGSQL.5432")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let sockets = Arc::new(Mutex::new(Vec::new()));
        let fired = Arc::new(AtomicBool::new(false));
        let commits = Arc::new(AtomicUsize::new(0));
        let thread = {
            let stop = stop.clone();
            let sockets = sockets.clone();
            let fired = fired.clone();
            std::thread::spawn(move || {
                let mut workers = Vec::new();
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut client, _)) = listener.accept() else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    if matches!(fault, Fault::AfterCommitUnavailable)
                        && fired.load(Ordering::SeqCst)
                    {
                        let _ = client.shutdown(Shutdown::Both);
                        continue;
                    }
                    let mut server = Upstream::connect(&host).unwrap();
                    sockets.lock().unwrap().push(client.try_clone().unwrap());
                    let fired = fired.clone();
                    let commits = commits.clone();
                    workers.push(std::thread::spawn(move || {
                        let result = (|| -> std::io::Result<()> {
                            let mut size = [0u8; 4];
                            client.read_exact(&mut size)?;
                            let mut startup = vec![0; u32::from_be_bytes(size) as usize - 4];
                            client.read_exact(&mut startup)?;
                            server.write_all(&size)?;
                            server.write_all(&startup)?;
                            let suppress = Arc::new(AtomicBool::new(false));
                            let mut backend = server.try_clone()?;
                            let mut output = client.try_clone()?;
                            let suppress_backend = suppress.clone();
                            let fired_backend = fired.clone();
                            let reader = std::thread::spawn(move || {
                                while let Ok(bytes) = frame(&mut backend) {
                                    // This scratch plan builds just one index. Match the
                                    // server's completion, not the query or ReadyForQuery.
                                    if matches!(fault, Fault::AfterCreateIndex)
                                        && bytes[0] == b'C'
                                        && &bytes[5..] == b"CREATE INDEX\0"
                                        && !fired_backend.swap(true, Ordering::SeqCst)
                                    {
                                        break; // never forward the build acknowledgement
                                    }
                                    if suppress_backend.load(Ordering::SeqCst) {
                                        // A pipelined Close/Sync can still have a pending
                                        // ReadyForQuery. Wait for this COMMIT's own
                                        // CommandComplete before severing the connection.
                                        if bytes[0] == b'C' && &bytes[5..] == b"COMMIT\0" {
                                            break;
                                        }
                                    } else if output.write_all(&bytes).is_err() {
                                        break;
                                    }
                                }
                                let _ = backend.shutdown(Shutdown::Both);
                                let _ = output.shutdown(Shutdown::Both);
                            });
                            while let Ok(bytes) = frame(&mut client) {
                                let is_commit = bytes[0] == b'Q' && &bytes[5..] == b"COMMIT\0";
                                let is_rollback = bytes[0] == b'Q' && &bytes[5..] == b"ROLLBACK\0";
                                let selected = is_commit
                                    && commits.fetch_add(1, Ordering::SeqCst) + 1 == commit;
                                let hit = match fault {
                                    Fault::BeforeCommit
                                    | Fault::AfterCommit
                                    | Fault::AfterCommitUnavailable => selected,
                                    Fault::Rollback => is_rollback,
                                    Fault::AfterCreateIndex => false, // backend-driven fault
                                };
                                if hit && !fired.swap(true, Ordering::SeqCst) {
                                    if matches!(
                                        fault,
                                        Fault::AfterCommit | Fault::AfterCommitUnavailable
                                    ) {
                                        suppress.store(true, Ordering::SeqCst);
                                    } else {
                                        break;
                                    }
                                }
                                if server.write_all(&bytes).is_err() {
                                    break;
                                }
                            }
                            let _ = server.shutdown(Shutdown::Both);
                            let _ = client.shutdown(Shutdown::Both);
                            reader.join().unwrap();
                            Ok(())
                        })();
                        let _ = result;
                    }));
                }
                for socket in sockets.lock().unwrap().iter() {
                    let _ = socket.shutdown(Shutdown::Both);
                }
                for worker in workers {
                    worker.join().unwrap();
                }
            })
        };
        Self {
            directory,
            stop,
            sockets,
            thread: Some(thread),
            fired,
        }
    }
    pub fn host(&self) -> &str {
        self.directory.path().to_str().unwrap()
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for s in self.sockets.lock().unwrap().iter() {
            let _ = s.shutdown(Shutdown::Both);
        }
        self.thread.take().unwrap().join().unwrap();
    }
}

pub(super) fn proxied_scratch_plan(
    f: &super::Fixture,
    scratch: &super::native_scratch::NativeScratch,
    upstream: &std::path::Path,
    fault: Fault,
) -> (Proxy, std::process::Command) {
    let proxy = Proxy::for_host(fault, 1, upstream.to_str().unwrap().into());
    let mut cmd = scratch.process(f, "plan");
    let host = proxy.host().replace('\\', "\\\\").replace('\'', "\\'");
    let name = &f.scratch;
    cmd.env(
        "DB_SCRATCH",
        format!("host='{host}' user={name} dbname={name} sslmode=disable"),
    );
    (proxy, cmd)
}

#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn isolated_prepare_lost_commit_ack_requires_external_cleanup() {
    use super::native_scratch::{assert_discarded, report, NativeScratch};

    let f = super::Fixture::new();
    f.sql("0001_table.sql", "CREATE TABLE app.t(id integer);");
    f.sql(
        "0002_index.sql",
        "CREATE INDEX CONCURRENTLY t_idx ON app.t(id);",
    );

    // No disposal instruction for an unverified (here, same-cluster) database.
    let refused = f.command("plan");
    assert!(super::code(&refused, "DB_REPLAY_ISOLATION_REQUIRED"));
    assert_eq!(refused["scratch"]["cleanup"], "not_needed");

    let scratch = NativeScratch::new(&f);
    let storage = scratch.storage();
    let upstream = storage.2.join("socket");
    let (proxy, mut cmd) = proxied_scratch_plan(&f, &scratch, &upstream, Fault::AfterCommit);
    let r = report(&mut cmd);
    assert!(proxy.fired.load(Ordering::SeqCst));
    assert!(super::code(&r, "DB_POSTGRES"));
    assert_eq!(r["plan"]["classification"], "blocked");
    // Independently observe the prepared schemas, but no migration: the
    // server committed preparation even though the CLI never saw the ACK.
    let prepared: bool = scratch
        .observer()
        .query_one(
            "SELECT to_regclass('_ntnt_replay.replay_v1') IS NOT NULL
            AND to_regnamespace('app') IS NOT NULL AND to_regclass('app.t') IS NULL",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(
        prepared,
        "expected the committed marker and managed schema only"
    );
    assert_eq!(r["scratch"]["cleanup"], "external");
    drop(proxy);
    drop(scratch);
    assert_discarded(storage);
}
