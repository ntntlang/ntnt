//! A Redis/Valkey stand-in for unit tests of connection deadlines. Each
//! connection is either served or stalled at a chosen point, and the server
//! reports what happened in order, so a test waits for events, never durations.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Only a hang reaches this; every exchange with the stand-in takes milliseconds.
pub(crate) const HANG_GUARD: Duration = Duration::from_secs(60);

/// What the stand-in does with one connection.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Plan {
    /// Answer commands: PING, GET (always nil), SCAN (always empty), QUIT
    /// (then close) and `+OK` for anything else.
    Serve,
    /// Read the client's first bytes (a TLS ClientHello) and never answer.
    StallHandshake,
    /// Complete TLS when the stand-in has a certificate, read the first command
    /// (a connection setup command) and never answer it.
    StallSetup,
}

#[derive(Debug)]
pub(crate) enum Event {
    /// Connection `connection` sent `first` (0x16 starts a TLS record, `*` a
    /// RESP command) and got no answer. `release` closes it from the server.
    Stalled {
        connection: usize,
        first: u8,
        release: Release,
    },
    /// The client closed stalled connection `connection`: it gave up.
    ClosedByClient { connection: usize },
    /// The stand-in closed stalled connection `connection` on release.
    Released { connection: usize },
    /// Served connection `connection` ended after these commands (upper case).
    Served {
        connection: usize,
        commands: Vec<String>,
    },
}

/// Closes a stalled connection from the server side.
#[derive(Debug)]
pub(crate) struct Release {
    stream: TcpStream,
    released: Arc<AtomicBool>,
}

impl Release {
    pub(crate) fn close(self) {
        self.released.store(true, Ordering::SeqCst);
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

pub(crate) struct StandIn {
    pub(crate) port: u16,
    events: mpsc::Receiver<Event>,
}

impl StandIn {
    /// The next event, in the order the stand-in saw them.
    pub(crate) fn next(&self) -> Event {
        self.events
            .recv_timeout(HANG_GUARD)
            .unwrap_or_else(|error| panic!("the Redis stand-in reported nothing: {error}"))
    }
}

/// Start a stand-in on a loopback port it picks itself. Connection `n` follows
/// `plans[n]`; later connections follow the last plan. With `tls`, every
/// connection is TLS (a [`Plan::StallHandshake`] connection never gets that far).
pub(crate) fn start(plans: Vec<Plan>, tls: Option<Arc<rustls::ServerConfig>>) -> StandIn {
    assert!(!plans.is_empty(), "the stand-in needs a plan");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (report, events) = mpsc::channel();
    std::thread::spawn(move || {
        for (connection, stream) in listener.incoming().enumerate() {
            let Ok(stream) = stream else { continue };
            let plan = plans[connection.min(plans.len() - 1)];
            let (tls, report) = (tls.clone(), report.clone());
            std::thread::spawn(move || run(connection, plan, stream, tls, report));
        }
    });
    StandIn { port, events }
}

/// A server certificate for `127.0.0.1` from a throwaway CA: the server
/// configuration and the CA certificate a client must trust.
pub(crate) fn tls_identity() -> (
    Arc<rustls::ServerConfig>,
    rustls::pki_types::CertificateDer<'static>,
) {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec!["127.0.0.1".to_string()])
        .unwrap()
        .signed_by(&leaf_key, &ca, &ca_key)
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
    )
    .unwrap();
    (Arc::new(config), ca.der().clone())
}

trait Transport: Read + Write + Send {}
impl<T: Read + Write + Send> Transport for T {}

fn run(
    connection: usize,
    plan: Plan,
    stream: TcpStream,
    tls: Option<Arc<rustls::ServerConfig>>,
    report: mpsc::Sender<Event>,
) {
    let released = Arc::new(AtomicBool::new(false));
    let release = Release {
        stream: stream.try_clone().unwrap(),
        released: Arc::clone(&released),
    };
    let transport: Box<dyn Transport> = match (plan, tls) {
        (Plan::StallHandshake, _) | (_, None) => Box::new(stream),
        (_, Some(config)) => Box::new(rustls::StreamOwned::new(
            rustls::ServerConnection::new(config).unwrap(),
            stream,
        )),
    };
    let mut reader = BufReader::new(transport);
    if let Plan::Serve = plan {
        let commands = serve(&mut reader);
        // Closed before it is reported, so the report means the connection is gone.
        drop(reader);
        let _ = report.send(Event::Served {
            connection,
            commands,
        });
        return;
    }
    // Over TLS this read completes the handshake first.
    let mut first = [0u8; 1];
    if reader.read_exact(&mut first).is_err() {
        let _ = report.send(Event::ClosedByClient { connection });
        return;
    }
    let _ = report.send(Event::Stalled {
        connection,
        first: first[0],
        release,
    });
    // Never answer: read until the client gives up or the test releases it.
    let mut sink = [0u8; 4096];
    while matches!(reader.read(&mut sink), Ok(n) if n > 0) {}
    drop(reader);
    let _ = report.send(if released.load(Ordering::SeqCst) {
        Event::Released { connection }
    } else {
        Event::ClosedByClient { connection }
    });
}

/// Answer RESP commands until the client closes or sends QUIT.
fn serve(reader: &mut BufReader<Box<dyn Transport>>) -> Vec<String> {
    let mut commands = Vec::new();
    while let Some(name) = read_command(reader) {
        let reply: &[u8] = match name.as_str() {
            "PING" => b"+PONG\r\n",
            "GET" => b"$-1\r\n",
            "SCAN" => b"*2\r\n$1\r\n0\r\n*0\r\n",
            _ => b"+OK\r\n",
        };
        let quit = name == "QUIT";
        commands.push(name);
        if reader.get_mut().write_all(reply).is_err() || quit {
            break;
        }
    }
    commands
}

/// The name of the next RESP command, upper case; `None` at the end of input.
fn read_command(reader: &mut impl BufRead) -> Option<String> {
    let mut line = String::new();
    reader.read_line(&mut line).ok().filter(|n| *n > 0)?;
    let count: usize = line.trim_end().strip_prefix('*')?.parse().ok()?;
    let mut name = None;
    for _ in 0..count {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let length: usize = line.trim_end().strip_prefix('$')?.parse().ok()?;
        let mut word = vec![0u8; length + 2];
        reader.read_exact(&mut word).ok()?;
        word.truncate(length);
        name.get_or_insert_with(|| String::from_utf8_lossy(&word).to_ascii_uppercase());
    }
    name
}
