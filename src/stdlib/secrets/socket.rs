//! Generic local secrets-agent protocol v1 over Unix domain sockets.
//!
//! Each lookup opens one Unix stream, writes exactly one newline-delimited JSON
//! request, half-closes the write side, reads one bounded newline-delimited JSON
//! response through EOF, and closes the stream. Requests contain only
//! `protocol`, a non-credential numeric `request_id`, `op = "get"`, and the
//! validated logical secret `name`. Responses echo `protocol` and `request_id`,
//! include the expected deployment authorization `scope`, and use one of:
//! `found`, `missing`, `access_denied`, `unavailable`, `invalid_request`, or
//! `invalid_configuration`. Only `found` includes `value`.
//!
//! Frames are limited to 64 KiB, values to 32 KiB, and the complete
//! connect/write/read attempt is bounded by one configured deadline. Unknown fields, extra frames,
//! version/request/scope mismatches, empty values, and malformed responses fail
//! closed without rendering response bytes.

use super::{
    validate_no_symlink_components, ProviderEndpointLabel, ProviderError, ProviderErrorKind,
    ProviderLookup, SecretProvider,
};
use serde::{Deserialize, Serialize};
use socket2::{Domain, SockAddr, Socket, Type};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const PROTOCOL_VERSION: u8 = 1;
const MAX_RESPONSE_SIZE: usize = 65_536;
const MAX_SECRET_SIZE: usize = 32_768;
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn remaining_until(deadline: Instant) -> std::io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "provider attempt deadline elapsed",
            )
        })
}

struct DeadlineWriter<'a> {
    stream: &'a mut UnixStream,
    deadline: Instant,
}

impl Write for DeadlineWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.stream
            .set_write_timeout(Some(remaining_until(self.deadline)?))?;
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream
            .set_write_timeout(Some(remaining_until(self.deadline)?))?;
        self.stream.flush()
    }
}

pub(super) struct SocketSecretProvider {
    path: PathBuf,
    endpoint: ProviderEndpointLabel,
    authorization_scope: String,
    timeout: Duration,
    trusted_root: Option<PathBuf>,
}

impl SocketSecretProvider {
    pub(super) fn new(
        path: PathBuf,
        endpoint: ProviderEndpointLabel,
        authorization_scope: String,
        timeout: Duration,
    ) -> Self {
        Self {
            path,
            endpoint,
            authorization_scope,
            timeout,
            trusted_root: None,
        }
    }

    pub(super) fn new_with_trusted_root(
        path: PathBuf,
        endpoint: ProviderEndpointLabel,
        authorization_scope: String,
        timeout: Duration,
        trusted_root: PathBuf,
    ) -> Self {
        Self {
            path,
            endpoint,
            authorization_scope,
            timeout,
            trusted_root: Some(trusted_root),
        }
    }

    fn error(&self, kind: ProviderErrorKind) -> ProviderError {
        ProviderError::new(kind, &self.endpoint)
    }

    fn connect(&self, deadline: Instant) -> std::result::Result<UnixStream, ProviderError> {
        if let Some(trusted_root) = &self.trusted_root {
            if self.path == *trusted_root
                || !self.path.starts_with(trusted_root)
                || validate_no_symlink_components(&self.path).is_err()
            {
                return Err(self.error(ProviderErrorKind::InvalidConfiguration));
            }
        }
        match std::fs::symlink_metadata(&self.path) {
            Ok(metadata)
                if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() =>
            {
                return Err(self.error(ProviderErrorKind::InvalidConfiguration));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(self.error(ProviderErrorKind::Unavailable));
            }
            Err(_) => return Err(self.error(ProviderErrorKind::InvalidConfiguration)),
        }
        let socket = Socket::new(Domain::UNIX, Type::STREAM, None)
            .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;
        let address = SockAddr::unix(&self.path)
            .map_err(|_| self.error(ProviderErrorKind::InvalidConfiguration))?;
        let connect_timeout =
            remaining_until(deadline).map_err(|_| self.error(ProviderErrorKind::Unavailable))?;
        socket
            .connect_timeout(&address, connect_timeout)
            .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;

        // SAFETY: ownership of the live stream descriptor moves from socket2 to
        // UnixStream exactly once; `socket` cannot close it after `into_raw_fd`.
        let stream = unsafe { UnixStream::from_raw_fd(socket.into_raw_fd()) };
        stream
            .set_nonblocking(false)
            .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;
        Ok(stream)
    }

    fn validate_response_header(
        &self,
        protocol: u8,
        response_request_id: u64,
        expected_request_id: u64,
        scope: &str,
    ) -> std::result::Result<(), ProviderError> {
        if protocol != PROTOCOL_VERSION || response_request_id != expected_request_id {
            return Err(self.error(ProviderErrorKind::InvalidConfiguration));
        }
        if scope != self.authorization_scope {
            return Err(self.error(ProviderErrorKind::AccessDenied));
        }
        Ok(())
    }

    fn write_request(
        &self,
        writer: &mut impl Write,
        name: &str,
    ) -> std::result::Result<u64, ProviderError> {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let request = SocketRequest {
            protocol: PROTOCOL_VERSION,
            request_id,
            op: "get",
            name,
            scope: &self.authorization_scope,
        };
        serde_json::to_writer(&mut *writer, &request)
            .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;
        Ok(request_id)
    }

    fn read_response_frame(
        &self,
        stream: &mut UnixStream,
        deadline: Instant,
    ) -> std::result::Result<Zeroizing<Vec<u8>>, ProviderError> {
        let mut response = Zeroizing::new(Vec::with_capacity(1024));
        let mut chunk = Zeroizing::new([0_u8; 4096]);

        loop {
            let read_timeout = remaining_until(deadline)
                .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;
            stream
                .set_read_timeout(Some(read_timeout))
                .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;
            let remaining = MAX_RESPONSE_SIZE + 1 - response.len();
            let read_size = remaining.min(chunk.len());
            match stream.read(&mut chunk[..read_size]) {
                Ok(0) if response.len() >= MAX_RESPONSE_SIZE => {
                    return Err(self.error(ProviderErrorKind::InvalidConfiguration));
                }
                Ok(0) => return Err(self.error(ProviderErrorKind::Unavailable)),
                Ok(bytes_read) => {
                    let bytes = &chunk[..bytes_read];
                    if bytes.contains(&b'\r') {
                        return Err(self.error(ProviderErrorKind::InvalidConfiguration));
                    }
                    if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
                        response.extend_from_slice(&bytes[..=newline]);
                        if newline + 1 != bytes_read || response.len() > MAX_RESPONSE_SIZE {
                            return Err(self.error(ProviderErrorKind::InvalidConfiguration));
                        }

                        let mut trailing = Zeroizing::new([0_u8; 1]);
                        stream
                            .set_nonblocking(true)
                            .map_err(|_| self.error(ProviderErrorKind::InvalidConfiguration))?;
                        loop {
                            match stream.read(&mut trailing[..]) {
                                Ok(0) => return Ok(response),
                                Err(error)
                                    if matches!(
                                        error.kind(),
                                        std::io::ErrorKind::ConnectionReset
                                            | std::io::ErrorKind::ConnectionAborted
                                    ) =>
                                {
                                    return Ok(response);
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                    let remaining = remaining_until(deadline).map_err(|_| {
                                        self.error(ProviderErrorKind::InvalidConfiguration)
                                    })?;
                                    std::thread::sleep(remaining.min(Duration::from_millis(1)));
                                }
                                Ok(_) | Err(_) => {
                                    return Err(self.error(ProviderErrorKind::InvalidConfiguration));
                                }
                            }
                        }
                    }

                    response.extend_from_slice(bytes);
                    if response.len() > MAX_RESPONSE_SIZE {
                        return Err(self.error(ProviderErrorKind::InvalidConfiguration));
                    }
                }
                Err(error)
                    if response.len() >= MAX_RESPONSE_SIZE
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                {
                    return Err(self.error(ProviderErrorKind::InvalidConfiguration));
                }
                Err(_) => return Err(self.error(ProviderErrorKind::Unavailable)),
            }
        }
    }

    fn decode_response(
        &self,
        body: &[u8],
        request_id: u64,
    ) -> std::result::Result<ProviderLookup, ProviderError> {
        let parsed: SocketResponse = serde_json::from_slice(body)
            .map_err(|_| self.error(ProviderErrorKind::InvalidConfiguration))?;

        match parsed {
            SocketResponse::Found {
                protocol,
                request_id: response_request_id,
                scope,
                value,
            } => {
                self.validate_response_header(protocol, response_request_id, request_id, &scope)?;
                if value.is_empty() || value.len() > MAX_SECRET_SIZE {
                    return Err(self.error(ProviderErrorKind::InvalidConfiguration));
                }
                Ok(ProviderLookup::Found(value))
            }
            SocketResponse::Missing {
                protocol,
                request_id: response_request_id,
                scope,
            } => {
                self.validate_response_header(protocol, response_request_id, request_id, &scope)?;
                Ok(ProviderLookup::Missing)
            }
            SocketResponse::AccessDenied {
                protocol,
                request_id: response_request_id,
                scope,
            } => {
                self.validate_response_header(protocol, response_request_id, request_id, &scope)?;
                Err(self.error(ProviderErrorKind::AccessDenied))
            }
            SocketResponse::Unavailable {
                protocol,
                request_id: response_request_id,
                scope,
            } => {
                self.validate_response_header(protocol, response_request_id, request_id, &scope)?;
                Err(self.error(ProviderErrorKind::Unavailable))
            }
            SocketResponse::InvalidRequest {
                protocol,
                request_id: response_request_id,
                scope,
            } => {
                self.validate_response_header(protocol, response_request_id, request_id, &scope)?;
                Err(self.error(ProviderErrorKind::InvalidRequest))
            }
            SocketResponse::InvalidConfiguration {
                protocol,
                request_id: response_request_id,
                scope,
            } => {
                self.validate_response_header(protocol, response_request_id, request_id, &scope)?;
                Err(self.error(ProviderErrorKind::InvalidConfiguration))
            }
        }
    }
}

#[derive(Serialize)]
struct SocketRequest<'a> {
    protocol: u8,
    request_id: u64,
    op: &'static str,
    name: &'a str,
    scope: &'a str,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum SocketResponse {
    Found {
        protocol: u8,
        request_id: u64,
        scope: String,
        value: Zeroizing<String>,
    },
    Missing {
        protocol: u8,
        request_id: u64,
        scope: String,
    },
    AccessDenied {
        protocol: u8,
        request_id: u64,
        scope: String,
    },
    Unavailable {
        protocol: u8,
        request_id: u64,
        scope: String,
    },
    InvalidRequest {
        protocol: u8,
        request_id: u64,
        scope: String,
    },
    InvalidConfiguration {
        protocol: u8,
        request_id: u64,
        scope: String,
    },
}

impl SecretProvider for SocketSecretProvider {
    fn endpoint(&self) -> &ProviderEndpointLabel {
        &self.endpoint
    }

    fn authorization_scope(&self) -> &str {
        &self.authorization_scope
    }

    fn lookup(&self, name: &str) -> std::result::Result<ProviderLookup, ProviderError> {
        let deadline = Instant::now() + self.timeout;
        let mut stream = self.connect(deadline)?;

        let request_id = self.write_request(
            &mut DeadlineWriter {
                stream: &mut stream,
                deadline,
            },
            name,
        )?;

        stream
            .shutdown(Shutdown::Write)
            .map_err(|_| self.error(ProviderErrorKind::Unavailable))?;

        let response = self.read_response_frame(&mut stream, deadline)?;
        self.decode_response(&response[..response.len() - 1], request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        configured_provider_group_from_values, ProviderEndpointLabel, ProviderErrorKind,
        ProviderLookup, SecretProvider,
    };
    use super::SocketSecretProvider;
    use serde_json::Value as JsonValue;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    const SECRET_CANARY: &str = "SOCKET_SECRET_CANARY_DO_NOT_DISCLOSE";
    const REQUEST_ID_PLACEHOLDER: &str = "__REQUEST_ID__";
    static SOCKET_TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn socket_path(label: &str) -> PathBuf {
        let counter = SOCKET_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        PathBuf::from("/tmp")
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!(
                "ntnt-sp-{label}-{}-{counter}.sock",
                std::process::id()
            ))
    }

    fn serve_responses(
        label: &str,
        responses: Vec<Vec<u8>>,
    ) -> (PathBuf, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
        let path = socket_path(label);
        let listener = UnixListener::bind(&path).expect("bind fixture socket");
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            for mut response in responses {
                let (mut stream, _) = listener.accept().expect("accept provider request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .expect("set fixture read timeout");
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .expect("set fixture write timeout");

                let mut request = String::new();
                BufReader::new(&stream)
                    .read_line(&mut request)
                    .expect("read provider request");
                let mut trailing_request = [0_u8; 1];
                assert_eq!(
                    stream
                        .read(&mut trailing_request)
                        .expect("read request EOF"),
                    0,
                    "provider request must half-close after one frame"
                );
                if let Ok(text) = String::from_utf8(response.clone()) {
                    if text.contains(REQUEST_ID_PLACEHOLDER) {
                        let parsed: JsonValue =
                            serde_json::from_str(&request).expect("valid provider request");
                        let request_id = parsed["request_id"].as_u64().expect("numeric request id");
                        response = text
                            .replace(
                                &format!("\"{REQUEST_ID_PLACEHOLDER}\""),
                                &request_id.to_string(),
                            )
                            .into_bytes();
                    }
                }
                request_tx.send(request).expect("capture request");
                stream
                    .write_all(&response)
                    .expect("write provider response");
                stream
                    .shutdown(std::net::Shutdown::Write)
                    .expect("close fixture response");
            }
        });
        (path, request_rx, server)
    }

    fn serve_response(
        label: &str,
        response: Vec<u8>,
    ) -> (PathBuf, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
        serve_responses(label, vec![response])
    }

    fn provider(path: PathBuf) -> SocketSecretProvider {
        SocketSecretProvider::new(
            path,
            ProviderEndpointLabel::socket(1),
            "deployment-a".to_string(),
            Duration::from_millis(250),
        )
    }

    #[test]
    fn socket_provider_rechecks_trusted_root_parent_symlinks_at_connect_time() {
        let root = socket_path("trusted-root");
        std::fs::create_dir_all(&root).expect("create trusted root fixture");
        let actual = root.join("actual");
        std::fs::create_dir_all(&actual).expect("create actual socket directory");
        let actual_socket = actual.join("agent.sock");
        let _listener = UnixListener::bind(&actual_socket).expect("bind actual socket");
        let symlinked_parent = root.join("link");
        std::os::unix::fs::symlink(&actual, &symlinked_parent).expect("create parent symlink");

        let Err(error) = SocketSecretProvider::new_with_trusted_root(
            symlinked_parent.join("agent.sock"),
            ProviderEndpointLabel::socket(1),
            "deployment-a".to_string(),
            Duration::from_millis(50),
            root.clone(),
        )
        .lookup("API_KEY") else {
            panic!("symlinked parent must fail closed");
        };
        assert_eq!(error.kind, ProviderErrorKind::InvalidConfiguration);

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn socket_provider_rejects_regular_files_and_symlinks_at_connect_time() {
        let regular_path = socket_path("regular-file");
        std::fs::write(&regular_path, b"not a socket").expect("write regular fixture");
        let Err(regular_error) = provider(regular_path.clone()).lookup("API_KEY") else {
            panic!("regular files must not be used as provider sockets");
        };
        assert_eq!(regular_error.kind, ProviderErrorKind::InvalidConfiguration);
        std::fs::remove_file(regular_path).ok();

        let target_path = socket_path("symlink-target");
        let _listener = UnixListener::bind(&target_path).expect("bind target socket");
        let symlink_path = socket_path("symlink-path");
        std::os::unix::fs::symlink(&target_path, &symlink_path).expect("create socket symlink");
        let Err(symlink_error) = provider(symlink_path.clone()).lookup("API_KEY") else {
            panic!("symlink socket path must fail closed");
        };
        assert_eq!(symlink_error.kind, ProviderErrorKind::InvalidConfiguration);
        std::fs::remove_file(symlink_path).ok();
        std::fs::remove_file(target_path).ok();
    }

    #[test]
    fn configured_socket_provider_retries_transient_failures_then_fails_over() {
        let unavailable =
            b"{\"protocol\":1,\"request_id\":\"__REQUEST_ID__\",\"status\":\"unavailable\",\"scope\":\"deployment-a\"}\n".to_vec();
        let found = format!(
            "{}\n",
            serde_json::json!({
                "protocol": 1,
                "request_id": REQUEST_ID_PLACEHOLDER,
                "status": "found",
                "scope": "deployment-a",
                "value": SECRET_CANARY,
            })
        )
        .into_bytes();
        let (first_path, first_requests, first_server) =
            serve_responses("failover-first", vec![unavailable.clone(), unavailable]);
        let (second_path, second_requests, second_server) =
            serve_response("failover-second", found);
        let endpoints = format!("{},{}", first_path.display(), second_path.display());

        let group = configured_provider_group_from_values(
            "unix-socket",
            Some(&endpoints),
            Some("deployment-a"),
            Some("1000"),
            false,
        )
        .expect("valid socket provider group");
        let secret = group
            .lookup("API_KEY")
            .expect("failover lookup")
            .expect("secret found");
        assert_eq!(secret.expose(), SECRET_CANARY);
        assert_eq!(first_requests.iter().count(), 2);
        assert_eq!(second_requests.iter().count(), 1);

        first_server.join().expect("first fixture server");
        second_server.join().expect("second fixture server");
        std::fs::remove_file(first_path).ok();
        std::fs::remove_file(second_path).ok();
    }

    #[test]
    fn socket_provider_classifies_incomplete_eof_as_unavailable() {
        for (label, response) in [
            ("empty-eof", b"".as_slice()),
            ("partial-eof", b"{\"protocol\":1".as_slice()),
        ] {
            let (path, _requests, server) = serve_response(label, response.to_vec());
            let Err(error) = provider(path.clone()).lookup("API_KEY") else {
                panic!("incomplete EOF must fail");
            };
            assert_eq!(error.kind, ProviderErrorKind::Unavailable);
            server.join().expect("incomplete EOF fixture");
            std::fs::remove_file(path).ok();
        }
    }

    #[test]
    fn socket_provider_treats_complete_frame_without_eof_as_terminal_malformed() {
        let path = socket_path("complete-without-eof");
        let listener = UnixListener::bind(&path).expect("bind fixture socket");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept fixture request");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("set fixture timeout");
            let mut request = String::new();
            BufReader::new(&stream)
                .read_line(&mut request)
                .expect("read request");
            let request: JsonValue =
                serde_json::from_str(request.trim_end()).expect("request JSON");
            let response = serde_json::json!({
                "protocol": 1,
                "request_id": request["request_id"],
                "status": "found",
                "scope": "deployment-a",
                "value": SECRET_CANARY,
            });
            writeln!(stream, "{response}").expect("write complete response");
            stream.flush().expect("flush response");
            std::thread::sleep(Duration::from_millis(150));
        });

        let Err(error) = SocketSecretProvider::new(
            path.clone(),
            ProviderEndpointLabel::socket(1),
            "deployment-a".to_string(),
            Duration::from_millis(30),
        )
        .lookup("API_KEY") else {
            panic!("complete frame without EOF must fail");
        };
        assert_eq!(error.kind, ProviderErrorKind::InvalidConfiguration);

        server.join().expect("fixture server");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn socket_provider_bounds_connect_and_read_failures() {
        let missing_path = socket_path("missing-endpoint");
        let started = std::time::Instant::now();
        let Err(connect_error) = SocketSecretProvider::new(
            missing_path,
            ProviderEndpointLabel::socket(1),
            "deployment-a".to_string(),
            Duration::from_millis(30),
        )
        .lookup("API_KEY") else {
            panic!("missing endpoint must be unavailable");
        };
        assert_eq!(connect_error.kind, ProviderErrorKind::Unavailable);
        assert!(started.elapsed() < Duration::from_secs(1));

        let path = socket_path("silent-endpoint");
        let listener = UnixListener::bind(&path).expect("bind silent fixture");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept silent request");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("set fixture timeout");
            let mut request = String::new();
            BufReader::new(&stream)
                .read_line(&mut request)
                .expect("read request");
            std::thread::sleep(Duration::from_millis(150));
        });

        let started = std::time::Instant::now();
        let Err(read_error) = SocketSecretProvider::new(
            path.clone(),
            ProviderEndpointLabel::socket(1),
            "deployment-a".to_string(),
            Duration::from_millis(30),
        )
        .lookup("API_KEY") else {
            panic!("silent endpoint must time out");
        };
        assert_eq!(read_error.kind, ProviderErrorKind::Unavailable);
        assert!(started.elapsed() < Duration::from_secs(1));

        server.join().expect("silent fixture server");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn socket_provider_applies_one_deadline_to_slow_drip_responses() {
        let path = socket_path("slow-drip");
        let listener = UnixListener::bind(&path).expect("bind slow-drip fixture");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept slow-drip request");
            let mut request = String::new();
            BufReader::new(&stream)
                .read_line(&mut request)
                .expect("read request");
            let request: JsonValue =
                serde_json::from_str(request.trim_end()).expect("request JSON");
            let response = format!(
                "{}\n",
                serde_json::json!({
                    "protocol": 1,
                    "request_id": request["request_id"],
                    "status": "found",
                    "scope": "deployment-a",
                    "value": SECRET_CANARY,
                })
            );
            for byte in response.bytes() {
                if stream.write_all(&[byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let started = std::time::Instant::now();
        let Err(error) = SocketSecretProvider::new(
            path.clone(),
            ProviderEndpointLabel::socket(1),
            "deployment-a".to_string(),
            Duration::from_millis(40),
        )
        .lookup("API_KEY") else {
            panic!("slow-drip response must exceed the attempt deadline");
        };
        assert_eq!(error.kind, ProviderErrorKind::Unavailable);
        assert!(started.elapsed() < Duration::from_millis(250));

        server.join().expect("slow-drip fixture");
        std::fs::remove_file(path).ok();
    }

    struct PartialWriteFailure {
        wrote_once: bool,
    }

    impl Write for PartialWriteFailure {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            if self.wrote_once {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "fixture write failure",
                ))
            } else {
                self.wrote_once = true;
                Ok(buffer.len().min(1))
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn socket_provider_classifies_partial_write_failure_as_unavailable() {
        let mut writer = PartialWriteFailure { wrote_once: false };
        let error = provider(PathBuf::from("/unused"))
            .write_request(&mut writer, "API_KEY")
            .expect_err("partial write must fail");
        assert_eq!(error.kind, ProviderErrorKind::Unavailable);
        assert!(!format!("{error:?}").contains("fixture write failure"));
    }

    #[test]
    fn socket_provider_rejects_malformed_or_ambiguous_frames() {
        let valid = format!(
            "{}",
            serde_json::json!({
                "protocol": 1,
                "request_id": REQUEST_ID_PLACEHOLDER,
                "status": "found",
                "scope": "deployment-a",
                "value": SECRET_CANARY,
            })
        );
        let frames = vec![
            ("invalid-json", b"not json\n".to_vec()),
            (
                "unknown-field",
                format!(
                    "{{\"protocol\":1,\"request_id\":\"__REQUEST_ID__\",\"status\":\"found\",\"scope\":\"deployment-a\",\"value\":\"{SECRET_CANARY}\",\"extra\":true}}\n"
                )
                .into_bytes(),
            ),
            (
                "wrong-protocol",
                format!(
                    "{{\"protocol\":2,\"request_id\":\"__REQUEST_ID__\",\"status\":\"found\",\"scope\":\"deployment-a\",\"value\":\"{SECRET_CANARY}\"}}\n"
                )
                .into_bytes(),
            ),
            ("no-newline", valid.clone().into_bytes()),
            ("carriage-return", format!("{valid}\r\n").into_bytes()),
            ("extra-newline", format!("{valid}\n\n").into_bytes()),
            (
                "multiple-messages",
                format!("{valid}\n{{\"status\":\"missing\"}}\n").into_bytes(),
            ),
            ("at-limit-no-newline", vec![b'x'; 65_536]),
            ("oversized", vec![b'x'; 65_537]),
        ];

        for (label, frame) in frames {
            let (path, _request_rx, server) = serve_response(label, frame);
            let Err(error) = provider(path.clone()).lookup("API_KEY") else {
                panic!("malformed frame '{label}' must fail closed");
            };
            let expected = if label == "no-newline" {
                ProviderErrorKind::Unavailable
            } else {
                ProviderErrorKind::InvalidConfiguration
            };
            assert_eq!(error.kind, expected, "frame fixture: {label}");
            let rendered = format!("{error:?}");
            assert!(!rendered.contains(SECRET_CANARY));
            assert!(!rendered.contains(label));

            server.join().expect("fixture server");
            std::fs::remove_file(path).ok();
        }
    }

    #[test]
    fn socket_provider_classifies_agent_failures_without_backend_text() {
        for (status, expected) in [
            ("unavailable", ProviderErrorKind::Unavailable),
            ("invalid_request", ProviderErrorKind::InvalidRequest),
            (
                "invalid_configuration",
                ProviderErrorKind::InvalidConfiguration,
            ),
        ] {
            let response = format!(
                "{}\n",
                serde_json::json!({
                    "protocol": 1,
                    "request_id": REQUEST_ID_PLACEHOLDER,
                    "status": status,
                    "scope": "deployment-a",
                })
            );
            let (path, _request_rx, server) = serve_response(status, response.into_bytes());

            let Err(error) = provider(path.clone()).lookup("API_KEY") else {
                panic!("agent failure must remain an error");
            };
            assert_eq!(error.kind, expected);
            assert!(!format!("{error:?}").contains(SECRET_CANARY));

            server.join().expect("fixture server");
            std::fs::remove_file(path).ok();
        }
    }

    #[test]
    fn socket_provider_rejects_empty_or_oversized_values() {
        let provider = provider(PathBuf::from("/tmp/unused-secrets-agent.sock"));
        for (label, value) in [
            ("empty-value", String::new()),
            ("oversized-value", "x".repeat(32_769)),
        ] {
            let response = serde_json::to_vec(&serde_json::json!({
                "protocol": 1,
                "request_id": 7,
                "status": "found",
                "scope": "deployment-a",
                "value": value,
            }))
            .expect("encode fixture response");

            let Err(error) = provider.decode_response(&response, 7) else {
                panic!("invalid value size must fail closed");
            };
            assert_eq!(
                error.kind,
                ProviderErrorKind::InvalidConfiguration,
                "value fixture: {label}"
            );
            assert!(!format!("{error:?}").contains(SECRET_CANARY));
        }
    }

    #[test]
    fn socket_provider_rejects_mismatched_request_id() {
        let response = format!(
            "{}\n",
            serde_json::json!({
                "protocol": 1,
                "request_id": 0,
                "status": "found",
                "scope": "deployment-a",
                "value": SECRET_CANARY,
            })
        );
        let (path, _request_rx, server) = serve_response("wrong-request-id", response.into_bytes());

        let Err(error) = provider(path.clone()).lookup("API_KEY") else {
            panic!("mismatched request id must fail closed");
        };
        assert_eq!(error.kind, ProviderErrorKind::InvalidConfiguration);
        assert!(!format!("{error:?}").contains(SECRET_CANARY));

        server.join().expect("fixture server");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn socket_provider_rejects_wrong_authorization_scope_before_returning_value() {
        let response = format!(
            "{}\n",
            serde_json::json!({
                "protocol": 1,
                "request_id": REQUEST_ID_PLACEHOLDER,
                "status": "found",
                "scope": "deployment-b",
                "value": SECRET_CANARY,
            })
        );
        let (path, _request_rx, server) = serve_response("wrong-scope", response.into_bytes());

        let Err(error) = provider(path.clone()).lookup("API_KEY") else {
            panic!("wrong authorization scope must fail closed");
        };
        assert_eq!(error.kind, ProviderErrorKind::AccessDenied);
        let rendered = format!("{error:?}");
        assert!(!rendered.contains("deployment-b"));
        assert!(!rendered.contains(SECRET_CANARY));

        server.join().expect("fixture server");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn socket_provider_maps_access_denied_to_terminal_class() {
        let response =
            b"{\"protocol\":1,\"request_id\":\"__REQUEST_ID__\",\"status\":\"access_denied\",\"scope\":\"deployment-a\"}\n".to_vec();
        let (path, _request_rx, server) = serve_response("secret-denied", response);

        let Err(error) = provider(path.clone()).lookup("API_KEY") else {
            panic!("access denial must be terminal");
        };
        assert_eq!(error.kind, ProviderErrorKind::AccessDenied);
        assert!(!format!("{error:?}").contains(SECRET_CANARY));

        server.join().expect("fixture server");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn socket_provider_maps_missing_without_exposing_backend_details() {
        let response =
            b"{\"protocol\":1,\"request_id\":\"__REQUEST_ID__\",\"status\":\"missing\",\"scope\":\"deployment-a\"}\n".to_vec();
        let (path, _request_rx, server) = serve_response("secret-missing", response);

        let lookup = provider(path.clone())
            .lookup("API_KEY")
            .expect("missing is a successful optional lookup");
        assert!(matches!(lookup, ProviderLookup::Missing));

        server.join().expect("fixture server");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn socket_provider_returns_matching_scope_secret() {
        let response = format!(
            "{}\n",
            serde_json::json!({
                "protocol": 1,
                "request_id": REQUEST_ID_PLACEHOLDER,
                "status": "found",
                "scope": "deployment-a",
                "value": SECRET_CANARY,
            })
        );
        let (path, request_rx, server) = serve_response("secret-found", response.into_bytes());

        let lookup = provider(path.clone())
            .lookup("API_KEY")
            .expect("socket lookup");
        match lookup {
            ProviderLookup::Found(value) => assert_eq!(value.as_str(), SECRET_CANARY),
            ProviderLookup::Missing => panic!("expected found secret"),
        }

        let mut request: JsonValue = serde_json::from_str(
            request_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("captured request")
                .trim_end(),
        )
        .expect("valid request JSON");
        assert!(request["request_id"].as_u64().is_some());
        request
            .as_object_mut()
            .expect("request object")
            .remove("request_id");
        assert_eq!(
            request,
            serde_json::json!({
                "protocol": 1,
                "op": "get",
                "name": "API_KEY",
                "scope": "deployment-a",
            })
        );

        server.join().expect("fixture server");
        std::fs::remove_file(path).ok();
    }
}
