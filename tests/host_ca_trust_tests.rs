//! Host trust is exercised through real CLI processes, never process-global env mutation.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct TlsFixture {
    directory: tempfile::TempDir,
    port: u16,
    stop: Arc<AtomicBool>,
    contacts: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl TlsFixture {
    fn new(hostname: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca = params.self_signed(&ca_key).unwrap();
        std::fs::write(directory.path().join("ca.pem"), ca.pem()).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = rcgen::CertificateParams::new(vec![hostname.to_string()])
            .unwrap()
            .signed_by(&key, &ca, &ca_key)
            .unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let contacts = Arc::new(AtomicUsize::new(0));
        let stopped = stop.clone();
        let accepted = contacts.clone();
        let config = Arc::new(config);
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        accepted.fetch_add(1, Ordering::AcqRel);
                        socket.set_nonblocking(false).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut stream = rustls::StreamOwned::new(
                            rustls::ServerConnection::new(config.clone()).unwrap(),
                            socket,
                        );
                        // tls_info completes the handshake without application data;
                        // probe_fetch sends one HTTP request after verifying the peer.
                        let mut request = Vec::new();
                        let mut buffer = [0; 1024];
                        while request.len() < 16_384 {
                            match stream.read(&mut buffer) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => request.extend_from_slice(&buffer[..n]),
                            }
                            if request.windows(4).any(|part| part == b"\r\n\r\n") {
                                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                                let _ = stream.flush();
                                break;
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("TLS fixture accept: {error}"),
                }
            }
        });
        Self {
            directory,
            port,
            stop,
            contacts,
            worker: Some(worker),
        }
    }

    fn command(&self, source: &str) -> Command {
        let script = self.directory.path().join("client.tnt");
        std::fs::write(&script, source).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
        command
            .arg("run")
            .arg(script)
            .env("NTNT_ENV", "development")
            .env("NTNT_NET_ALLOW_PRIVATE", "1")
            .env("NTNT_ALLOW_LOCALHOST", "1")
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR");
        command
    }

    fn run(&self, source: &str, certificates: &Path) -> Output {
        let empty_directory = self.directory.path().join("empty");
        std::fs::create_dir_all(&empty_directory).unwrap();
        self.command(source)
            .env("SSL_CERT_FILE", certificates)
            .env("SSL_CERT_DIR", empty_directory)
            .output()
            .unwrap()
    }

    fn probe_source(&self) -> String {
        format!(
            r#"
import {{ probe_fetch }} from "std/http"
match probe_fetch(map {{ "url": "https://127.0.0.1:{}", "timeout_ms": 2000 }}) {{
    Ok(response) => print(response["status"]),
    Err(error) => print("ERR:" + error)
}}
"#,
            self.port
        )
    }

    fn info_source(&self) -> String {
        format!(
            r#"
import {{ tls_info }} from "std/net"
match tls_info("127.0.0.1", map {{ "port": {}, "allow_private": true, "timeout_ms": 2000 }}) {{
    Ok(info) => print(info["valid"]),
    Err(error) => print("ERR:" + error)
}}
"#,
            self.port
        )
    }
}

impl Drop for TlsFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn host_ca_directory_is_used_without_a_certificate_file() {
    let fixture = TlsFixture::new("127.0.0.1");
    let directory = fixture.directory.path().join("roots");
    std::fs::create_dir(&directory).unwrap();
    std::fs::copy(
        fixture.directory.path().join("ca.pem"),
        directory.join("root.pem"),
    )
    .unwrap();
    let output = fixture
        .command(&format!(
            "{}\n{}",
            fixture.info_source(),
            fixture.probe_source()
        ))
        .env("SSL_CERT_DIR", directory)
        .output()
        .unwrap();
    assert_eq!(stdout(&output), "true\n200\n");
}

#[test]
fn restarted_process_uses_updated_ca_file_without_rebuilding_binary() {
    let fixture = TlsFixture::new("127.0.0.1");
    let path = fixture.directory.path().join("mutable-roots.pem");
    let unrelated = rcgen::generate_simple_self_signed(vec!["unrelated.invalid".into()]).unwrap();
    std::fs::write(&path, unrelated.cert.pem()).unwrap();
    assert_eq!(
        stdout(&fixture.run(&fixture.info_source(), &path)).trim(),
        "false"
    );
    std::fs::copy(fixture.directory.path().join("ca.pem"), &path).unwrap();
    assert_eq!(
        stdout(&fixture.run(&fixture.info_source(), &path)).trim(),
        "true"
    );
    assert_eq!(fixture.contacts.load(Ordering::Acquire), 2);
}

#[test]
fn host_ca_does_not_bypass_peer_or_hostname_verification() {
    for wrong_hostname in [false, true] {
        let fixture = TlsFixture::new(if wrong_hostname {
            "wrong.invalid"
        } else {
            "127.0.0.1"
        });
        let unrelated = TlsFixture::new("unrelated.invalid");
        let trusted = if wrong_hostname { &fixture } else { &unrelated };
        let source = format!("{}\n{}", fixture.info_source(), fixture.probe_source());
        let output = fixture.run(&source, &trusted.directory.path().join("ca.pem"));
        let text = stdout(&output);
        assert_eq!(text.lines().next(), Some("false"), "{text}");
        assert!(
            text.contains("ERR:HTTP TLS verification or handshake failed"),
            "{text}"
        );
        assert_eq!(
            fixture.contacts.load(Ordering::Acquire),
            2,
            "no TLS retry or verification bypass"
        );
    }
}

#[test]
fn unusable_host_trust_fails_before_target_contact() {
    for contents in [Some(""), Some("not a certificate\n"), None] {
        let fixture = TlsFixture::new("127.0.0.1");
        let path = fixture.directory.path().join("unusable.pem");
        if let Some(contents) = contents {
            std::fs::write(&path, contents).unwrap();
        }
        let source = format!("{}\n{}", fixture.info_source(), fixture.probe_source());
        let output = fixture.run(&source, &path);
        let text = stdout(&output);
        assert_eq!(
            text.matches("ERR:no usable host CA certificates").count(),
            2,
            "{text}"
        );
        assert_eq!(
            fixture.contacts.load(Ordering::Acquire),
            0,
            "missing local trust must not be reported as a target failure"
        );
    }
}

#[test]
fn tls_info_trusts_host_ca_file() {
    let fixture = TlsFixture::new("127.0.0.1");
    let output = fixture.run(
        &fixture.info_source(),
        &fixture.directory.path().join("ca.pem"),
    );
    assert_eq!(
        stdout(&output).trim(),
        "true",
        "host CA must authorize the private test chain"
    );
    assert_eq!(fixture.contacts.load(Ordering::Acquire), 1);
}

#[test]
fn https_probe_trusts_host_ca_file() {
    let fixture = TlsFixture::new("127.0.0.1");
    let output = fixture.run(
        &fixture.probe_source(),
        &fixture.directory.path().join("ca.pem"),
    );
    assert_eq!(
        stdout(&output).trim(),
        "200",
        "host CA must authorize the HTTPS peer"
    );
    assert_eq!(fixture.contacts.load(Ordering::Acquire), 1);
}
