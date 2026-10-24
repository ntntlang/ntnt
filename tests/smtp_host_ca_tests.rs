//! Real CLI regressions for SMTP's host CA store, without external mail servers.
//! The private CA is deliberately absent from bundled roots: both delivery tests
//! must fail when lettre ignores SSL_CERT_FILE / SSL_CERT_DIR.

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHILD_TIMEOUT: Duration = Duration::from_secs(20);
const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const BODY_PROOF: &str = "host-ca-delivery-proof";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TlsMode {
    Implicit,
    Starttls,
}

impl TlsMode {
    fn config_value(self) -> &'static str {
        match self {
            Self::Implicit => "implicit",
            Self::Starttls => "starttls",
        }
    }
}

fn certificates(server_name: &str) -> (Arc<ServerConfig>, String) {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.distinguished_name.push(
        DnType::CommonName,
        format!("SMTP host CA fixture for {server_name}"),
    );
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();

    // rcgen encodes 127.0.0.1 as an IP SAN, not a DNS SAN.
    let mut leaf_params = CertificateParams::new(vec![server_name.to_owned()]).unwrap();
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "SMTP loopback fixture");
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).unwrap();
    // Do not use/install a process-global provider: JWT also enables AWS-LC.
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.der().clone()],
                PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
            )
            .unwrap();
    (Arc::new(config), ca.pem())
}

#[derive(Debug, Default)]
struct SmtpReport {
    tcp_contacts: usize,
    starttls_requests: usize,
    tls_handshakes: usize,
    mail_commands: usize,
    data_commands: usize,
    messages: Vec<String>,
    errors: Vec<String>,
}

struct SmtpFixture {
    port: u16,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<SmtpReport>>,
}

impl SmtpFixture {
    fn start(mode: TlsMode, config: Arc<ServerConfig>, offer_starttls: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + CHILD_TIMEOUT + Duration::from_secs(5);
            let mut report = SmtpReport::default();
            loop {
                // Drain pending accepts even after stop, so a fast child cannot
                // hide a TCP contact by exiting before this thread is scheduled.
                match listener.accept() {
                    Ok((stream, _)) => {
                        report.tcp_contacts += 1;
                        let session_deadline =
                            deadline.min(Instant::now() + Duration::from_secs(8));
                        if let Err(error) = serve_smtp(
                            stream,
                            mode,
                            config.clone(),
                            offer_starttls,
                            session_deadline,
                            &mut report,
                        ) {
                            // Certificate rejection is expected in negative tests.
                            report.errors.push(error.to_string());
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if worker_stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                            break;
                        }
                        thread::sleep(POLL_INTERVAL);
                    }
                    Err(error) => {
                        report.errors.push(format!("accept: {error}"));
                        break;
                    }
                }
                if Instant::now() >= deadline {
                    break;
                }
            }
            report
        });
        Self {
            port,
            stop,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> SmtpReport {
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("SMTP fixture panicked")
    }
}

impl Drop for SmtpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // Join on assertion failures too; sockets and loops are bounded.
            let _ = worker.join();
        }
    }
}

fn check_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "SMTP fixture deadline",
        ))
    } else {
        Ok(())
    }
}

fn line(stream: &mut impl Read, deadline: Instant) -> io::Result<String> {
    // No buffered read-ahead across the plaintext -> TLS boundary.
    let mut bytes = Vec::new();
    loop {
        check_deadline(deadline)?;
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if bytes.len() > 65_536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SMTP line too long",
            ));
        }
        if bytes.ends_with(b"\r\n") {
            return String::from_utf8(bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
    }
}

fn reply(stream: &mut impl Write, text: &str) -> io::Result<()> {
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

fn expect_command(stream: &mut impl Read, prefix: &str, deadline: Instant) -> io::Result<()> {
    let command = line(stream, deadline)?;
    if !command.starts_with(prefix) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected {prefix:?}, received {command:?}"),
        ));
    }
    Ok(())
}

fn ehlo(
    stream: &mut (impl Read + Write),
    offer_starttls: bool,
    deadline: Instant,
) -> io::Result<()> {
    expect_command(stream, "EHLO ", deadline)?;
    // No AUTH extension or credentials are needed by this local fixture.
    reply(
        stream,
        if offer_starttls {
            "250-localhost\r\n250 STARTTLS\r\n"
        } else {
            "250 localhost\r\n"
        },
    )
}

fn upgrade_tls(
    mut stream: TcpStream,
    config: Arc<ServerConfig>,
    deadline: Instant,
    report: &mut SmtpReport,
) -> io::Result<StreamOwned<ServerConnection, TcpStream>> {
    let mut connection = ServerConnection::new(config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    while connection.is_handshaking() {
        check_deadline(deadline)?;
        connection.complete_io(&mut stream)?;
    }
    report.tls_handshakes += 1;
    Ok(StreamOwned::new(connection, stream))
}

fn deliver(
    stream: &mut (impl Read + Write),
    deadline: Instant,
    report: &mut SmtpReport,
) -> io::Result<()> {
    expect_command(stream, "MAIL FROM:<sender@example.test>", deadline)?;
    report.mail_commands += 1;
    reply(stream, "250 Sender accepted\r\n")?;
    expect_command(stream, "RCPT TO:<recipient@example.test>", deadline)?;
    reply(stream, "250 Recipient accepted\r\n")?;
    expect_command(stream, "DATA\r\n", deadline)?;
    report.data_commands += 1;
    reply(stream, "354 End with <CRLF>.<CRLF>\r\n")?;
    let mut message = String::new();
    loop {
        let next = line(stream, deadline)?;
        if next == ".\r\n" {
            break;
        }
        message.push_str(&next);
        if message.len() > 65_536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SMTP message too long",
            ));
        }
    }
    reply(stream, "250 Message queued by local fixture\r\n")?;
    report.messages.push(message);
    // A pooled transport can close at process exit without QUIT. Once DATA is
    // acknowledged, tolerate either EOF or a normal QUIT (and pool NOOPs).
    while let Ok(command) = line(stream, deadline) {
        match command.as_str() {
            "QUIT\r\n" => {
                reply(stream, "221 Bye\r\n")?;
                break;
            }
            "NOOP\r\n" => reply(stream, "250 OK\r\n")?,
            _ => break,
        }
    }
    Ok(())
}

fn serve_smtp(
    stream: TcpStream,
    mode: TlsMode,
    config: Arc<ServerConfig>,
    offer_starttls: bool,
    deadline: Instant,
    report: &mut SmtpReport,
) -> io::Result<()> {
    // Accepted sockets can inherit nonblocking mode on macOS/Windows.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(SOCKET_TIMEOUT))?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT))?;
    if mode == TlsMode::Implicit {
        let mut tls = upgrade_tls(stream, config, deadline, report)?;
        reply(&mut tls, "220 localhost SMTP fixture\r\n")?;
        ehlo(&mut tls, false, deadline)?;
        deliver(&mut tls, deadline, report)
    } else {
        let mut plain = stream;
        reply(&mut plain, "220 localhost SMTP fixture\r\n")?;
        ehlo(&mut plain, offer_starttls, deadline)?;
        if !offer_starttls {
            // Deliberately accept plaintext delivery if the client downgrades:
            // the required-STARTTLS test must catch that, not cause it to fail.
            return deliver(&mut plain, deadline, report);
        }
        expect_command(&mut plain, "STARTTLS\r\n", deadline)?;
        report.starttls_requests += 1;
        reply(&mut plain, "220 Ready to start TLS\r\n")?;
        let mut tls = upgrade_tls(plain, config, deadline, report)?;
        ehlo(&mut tls, false, deadline)?;
        deliver(&mut tls, deadline, report)
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Debug)]
struct CliOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_cli(dir: &Path, port: u16, mode: TlsMode, ca_file: &Path) -> CliOutput {
    let script = dir.join("send.tnt");
    fs::write(
        &script,
        format!(
            r#"
import {{ configure_email, send_email }} from "std/email"
configure_email(map {{
    "transport": "smtp", "host": "127.0.0.1", "port": {port},
    "tls": "{}", "from": "sender@example.test"
}})
let sent = send_email(map {{
    "to": "recipient@example.test", "subject": "Host CA trust regression",
    "text": "{BODY_PROOF}"
}}) otherwise {{ print("SMTP_REJECTED: #{{err}}") return }}
print("SMTP_SENT: " + sent["transport"])
"#,
            mode.config_value()
        ),
    )
    .unwrap();
    let empty_ca_dir = dir.join("empty-ca-directory");
    fs::create_dir(&empty_ca_dir).unwrap();
    let stdout_file = dir.join("stdout.txt");
    let stderr_file = dir.join("stderr.txt");
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .arg("run")
            .arg(&script)
            .current_dir(dir)
            .env("NTNT_ENV", "development")
            .env("NTNT_EMAIL_MODE", "smtp")
            .env("SSL_CERT_FILE", ca_file)
            .env("SSL_CERT_DIR", &empty_ca_dir)
            .stdin(Stdio::null())
            // Files avoid pipe-buffer deadlocks while polling a bounded child.
            .stdout(Stdio::from(File::create(&stdout_file).unwrap()))
            .stderr(Stdio::from(File::create(&stderr_file).unwrap()))
            .spawn()
            .expect("launch Cargo's ntnt integration-test binary"),
    );
    let deadline = Instant::now() + CHILD_TIMEOUT;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            child.0.kill().unwrap();
            break child.0.wait().unwrap();
        }
        thread::sleep(POLL_INTERVAL);
    };
    drop(child);
    let output = CliOutput {
        status,
        stdout: fs::read_to_string(stdout_file).unwrap(),
        stderr: fs::read_to_string(stderr_file).unwrap(),
    };
    assert!(
        !timed_out,
        "NTNT child exceeded {CHILD_TIMEOUT:?}: {output:?}"
    );
    output
}

fn rejection(output: &CliOutput) -> &str {
    assert!(
        output.status.success(),
        "CLI did not handle failure: {output:?}"
    );
    assert!(!output.stdout.contains("SMTP_SENT:"), "{output:?}");
    assert!(!output.stderr.contains("[EMAIL:log]"), "{output:?}");
    output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("SMTP_REJECTED: "))
        .unwrap_or_else(|| panic!("missing handled SMTP rejection: {output:?}"))
}

fn assert_delivery(mode: TlsMode) {
    let dir = tempfile::tempdir().unwrap();
    let (config, ca) = certificates("127.0.0.1");
    let ca_file = dir.path().join("ca.pem");
    fs::write(&ca_file, ca).unwrap();
    let fixture = SmtpFixture::start(mode, config, true);
    let output = run_cli(dir.path(), fixture.port, mode, &ca_file);
    let report = fixture.finish();
    assert!(output.status.success(), "{mode:?}: {output:?}; {report:?}");
    assert!(
        output.stdout.lines().any(|line| line == "SMTP_SENT: smtp"),
        "{mode:?}: private host CA was not trusted: {output:?}; {report:?}"
    );
    assert!(!output.stdout.contains("SMTP_REJECTED:"), "{output:?}");
    assert!(!output.stderr.contains("[EMAIL:log]"), "{output:?}");
    assert_eq!(report.tcp_contacts, 1, "{report:?}");
    assert_eq!(report.tls_handshakes, 1, "{report:?}");
    assert_eq!(
        report.starttls_requests,
        usize::from(mode == TlsMode::Starttls),
        "{report:?}"
    );
    assert_eq!(report.mail_commands, 1, "{report:?}");
    assert_eq!(report.data_commands, 1, "{report:?}");
    assert_eq!(report.messages.len(), 1, "{report:?}");
    assert!(report.messages[0].contains(BODY_PROOF), "{report:?}");
    assert!(
        report.messages[0].contains("Subject: Host CA trust regression"),
        "{report:?}"
    );
    assert!(report.errors.is_empty(), "{report:?}");
}

#[test]
fn smtp_implicit_tls_trusts_host_ca_and_completes_data() {
    assert_delivery(TlsMode::Implicit);
}

#[test]
fn smtp_required_starttls_trusts_host_ca_and_completes_data() {
    assert_delivery(TlsMode::Starttls);
}

#[test]
fn smtp_rejects_untrusted_ca_in_both_tls_modes() {
    for mode in [TlsMode::Implicit, TlsMode::Starttls] {
        let dir = tempfile::tempdir().unwrap();
        let (config, _) = certificates("127.0.0.1");
        let (_, unrelated_ca) = certificates("unrelated-ca.invalid");
        let ca_file = dir.path().join("unrelated-ca.pem");
        fs::write(&ca_file, unrelated_ca).unwrap();
        let fixture = SmtpFixture::start(mode, config, true);
        let output = run_cli(dir.path(), fixture.port, mode, &ca_file);
        let report = fixture.finish();
        assert!(
            rejection(&output).contains("UnknownIssuer"),
            "{mode:?}: {output:?}; {report:?}"
        );
        assert_tls_rejected(mode, &report);
    }
}

#[test]
fn smtp_rejects_wrong_hostname_in_both_tls_modes() {
    for mode in [TlsMode::Implicit, TlsMode::Starttls] {
        let dir = tempfile::tempdir().unwrap();
        let (config, ca) = certificates("wrong-host.invalid");
        let ca_file = dir.path().join("ca.pem");
        fs::write(&ca_file, ca).unwrap();
        let fixture = SmtpFixture::start(mode, config, true);
        let output = run_cli(dir.path(), fixture.port, mode, &ca_file);
        let report = fixture.finish();
        let error = rejection(&output).to_ascii_lowercase();
        assert!(
            error.contains("not valid for name") || error.contains("notvalidforname"),
            "{mode:?}: expected hostname rejection with a trusted CA: {output:?}; {report:?}"
        );
        assert_tls_rejected(mode, &report);
    }
}

fn assert_tls_rejected(mode: TlsMode, report: &SmtpReport) {
    assert!(report.tcp_contacts > 0, "{mode:?}: {report:?}");
    assert_eq!(
        report.starttls_requests,
        if mode == TlsMode::Starttls {
            report.tcp_contacts
        } else {
            0
        },
        "{report:?}"
    );
    assert_eq!(report.tls_handshakes, 0, "{report:?}");
    assert_eq!(report.mail_commands, 0, "{report:?}");
    assert_eq!(report.data_commands, 0, "{report:?}");
    assert!(report.messages.is_empty(), "{report:?}");
}

fn assert_unusable_store(contents: Option<&str>) {
    for mode in [TlsMode::Implicit, TlsMode::Starttls] {
        let dir = tempfile::tempdir().unwrap();
        let (config, _) = certificates("127.0.0.1");
        let ca_file = dir.path().join("unusable-ca.pem");
        if let Some(contents) = contents {
            fs::write(&ca_file, contents).unwrap();
        }
        let fixture = SmtpFixture::start(mode, config, true);
        let output = run_cli(dir.path(), fixture.port, mode, &ca_file);
        let report = fixture.finish();
        assert!(
            rejection(&output).contains("no usable host CA certificates"),
            "{mode:?}: expected clear TLS setup failure: {output:?}; {report:?}"
        );
        assert_eq!(
            report.tcp_contacts, 0,
            "{mode:?}: TLS setup contacted SMTP: {report:?}"
        );
    }
}

#[test]
fn smtp_empty_host_ca_store_fails_before_tcp_contact() {
    assert_unusable_store(Some(""));
}

#[test]
fn smtp_missing_host_ca_store_fails_before_tcp_contact() {
    assert_unusable_store(None);
}

#[test]
fn smtp_malformed_host_ca_store_fails_before_tcp_contact() {
    // A PEM certificate block whose decoded bytes are not an X.509 certificate.
    assert_unusable_store(Some(
        "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
    ));
}

#[test]
fn smtp_required_starttls_rejects_plaintext_only_server() {
    let dir = tempfile::tempdir().unwrap();
    let (config, ca) = certificates("127.0.0.1");
    let ca_file = dir.path().join("ca.pem");
    fs::write(&ca_file, ca).unwrap();
    let fixture = SmtpFixture::start(TlsMode::Starttls, config, false);
    let output = run_cli(dir.path(), fixture.port, TlsMode::Starttls, &ca_file);
    let report = fixture.finish();
    assert!(
        rejection(&output).to_ascii_lowercase().contains("starttls"),
        "{output:?}; {report:?}"
    );
    assert!(report.tcp_contacts > 0, "{report:?}");
    assert_eq!(report.starttls_requests, 0, "{report:?}");
    assert_eq!(report.tls_handshakes, 0, "{report:?}");
    assert_eq!(
        report.mail_commands, 0,
        "required STARTTLS downgraded: {report:?}"
    );
    assert_eq!(report.data_commands, 0, "{report:?}");
    assert!(report.messages.is_empty(), "{report:?}");
}
