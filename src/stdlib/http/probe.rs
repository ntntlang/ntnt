//! Fresh, single-contact HTTP monitoring transport.

use crate::interpreter::Value;
use axum::body::Body;
use hyper::{body::Body as HttpBody, Request};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::time::Duration;

type ProbeResult<T> = std::result::Result<T, String>;

const PROBE_LIMIT: usize = 16;
static PROBES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug)]
struct ProbeSlot {
    counter: &'static std::sync::atomic::AtomicUsize,
}

impl ProbeSlot {
    fn reserve() -> ProbeResult<Self> {
        Self::reserve_from(&PROBES, PROBE_LIMIT)
    }

    fn reserve_from(
        counter: &'static std::sync::atomic::AtomicUsize,
        limit: usize,
    ) -> ProbeResult<Self> {
        use std::sync::atomic::Ordering;

        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < limit).then_some(n + 1)
            })
            .map_err(|_| "HTTP probe worker busy")?;
        Ok(Self { counter })
    }
}

impl Drop for ProbeSlot {
    fn drop(&mut self) {
        self.counter
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[cfg(test)]
thread_local! {
    static TLS_ROOTS: std::cell::RefCell<Vec<rustls_pki_types::CertificateDer<'static>>> = const { std::cell::RefCell::new(Vec::new()) };
    static BEFORE_RESOLVE: std::cell::RefCell<Option<Box<dyn FnOnce() + Send>>> = const { std::cell::RefCell::new(None) };
    static BEFORE_CONNECT: std::cell::RefCell<Option<Box<dyn FnOnce() + Send>>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn fetch(opts: &HashMap<String, Value>) -> ProbeResult<Value> {
    // This deliberately narrow API does not expose credentials or Secret values.
    // Unsupported fetch options are rejected, never silently discarded/sent.
    if opts.values().any(Value::contains_secret) {
        return Err("probe_fetch does not accept Secret values".into());
    }
    for key in opts.keys() {
        if !matches!(
            key.as_str(),
            "url"
                | "method"
                | "timeout_ms"
                | "start_deadline_ms"
                | "start_monotonic_deadline_ms"
                | "redirect"
                | "follow_redirects"
        ) {
            return Err("Unsupported probe_fetch option".into());
        }
    }
    match opts.get("method") {
        None => {}
        Some(Value::String(method)) if method.eq_ignore_ascii_case("GET") => {}
        _ => return Err("probe_fetch only supports GET".into()),
    }
    if opts
        .get("redirect")
        .is_some_and(|v| !matches!(v, Value::String(s) if s == "manual"))
        || opts
            .get("follow_redirects")
            .is_some_and(|v| !matches!(v, Value::Bool(false)))
    {
        return Err("probe_fetch redirects are always manual".into());
    }
    let timeout = match opts.get("timeout_ms") {
        None => 30_000,
        Some(Value::Int(n)) if (1..=60_000).contains(n) => *n as u64,
        _ => return Err("timeout_ms must be an Int in 1..60000".into()),
    };
    let expires = std::time::Instant::now() + Duration::from_millis(timeout);
    let Some(Value::String(url)) = opts.get("url") else {
        return Err("probe_fetch requires url: String".into());
    };
    let deadline = crate::stdlib::send_deadline::SendDeadline::parse(Some(opts))?;
    let url = super::redirect_url(url)?;
    // Hold one process-wide admission slot from resolution through the joined I/O
    // worker so slow targets cannot create an unbounded number of runtimes/threads.
    let _probe_slot = ProbeSlot::reserve()?;
    let address = resolve_target(url.as_str(), expires)?;
    let uri: hyper::Uri = url.as_str().parse().map_err(|_| "Invalid probe URL")?;
    let request = Request::get(uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"))
        .header(
            "host",
            uri.authority().ok_or("Missing probe host")?.as_str(),
        )
        .header("connection", "close")
        .body(Body::empty())
        .map_err(|_| "Invalid probe request")?;
    let tls = if url.scheme() == "https" {
        let mut roots = rustls::RootCertStore::empty();
        roots.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
        #[cfg(test)]
        TLS_ROOTS.with(|extra| {
            roots.add_parsable_certificates(std::mem::take(&mut *extra.borrow_mut()))
        });
        let mut config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| "HTTP TLS configuration failed")?
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let name = url
            .host_str()
            .ok_or("Missing probe host")?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let name =
            rustls_pki_types::ServerName::try_from(name).map_err(|_| "Invalid TLS server name")?;
        Some((
            tokio_rustls::TlsConnector::from(std::sync::Arc::new(config)),
            name,
        ))
    } else {
        None
    };
    #[cfg(test)]
    let before_connect = BEFORE_CONNECT.with(|hook| hook.borrow_mut().take());
    let result = std::thread::Builder::new()
        .name("http-probe-io".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "HTTP runtime unavailable")?;
            runtime.block_on(async move {
                tokio::time::timeout_at(tokio::time::Instant::from_std(expires), async move {
                    let socket = socket2::Socket::new(
                        socket2::Domain::for_address(address),
                        socket2::Type::STREAM,
                        Some(socket2::Protocol::TCP),
                    )
                    .map_err(|_| "HTTP socket setup failed")?;
                    socket
                        .set_nonblocking(true)
                        .map_err(|_| "HTTP socket setup failed")?;
                    let sockaddr = socket2::SockAddr::from(address);
                    #[cfg(test)]
                    if let Some(hook) = before_connect {
                        hook();
                    }
                    let started = chrono::Utc::now().timestamp_millis();
                    // No resolution, allocation, await, or address fallback between this
                    // authorization and the nonblocking OS connect (first target contact).
                    if std::time::Instant::now() >= expires {
                        return Err("HTTP probe timeout".into());
                    }
                    deadline.check()?;
                    if let Err(error) = socket.connect(&sockaddr) {
                        #[cfg(unix)]
                        let pending = error.raw_os_error() == Some(libc::EINPROGRESS);
                        #[cfg(windows)]
                        let pending = error.kind() == std::io::ErrorKind::WouldBlock;
                        #[cfg(not(any(unix, windows)))]
                        let pending = error.kind() == std::io::ErrorKind::WouldBlock;
                        if !pending {
                            return Err("HTTP connect failed".into());
                        }
                    }
                    let stream = tokio::net::TcpStream::from_std(socket.into())
                        .map_err(|_| "HTTP socket registration failed")?;
                    stream.writable().await.map_err(|_| "HTTP connect failed")?;
                    if stream
                        .take_error()
                        .map_err(|_| "HTTP connect failed")?
                        .is_some()
                    {
                        return Err("HTTP connect failed".into());
                    }
                    if let Some((connector, name)) = tls {
                        let stream = connector
                            .connect(name, stream)
                            .await
                            .map_err(|_| "HTTP TLS verification or handshake failed")?;
                        exchange(stream, request, started, expires).await
                    } else {
                        exchange(stream, request, started, expires).await
                    }
                })
                .await
                .map_err(|_| "HTTP probe timeout".to_string())?
            })
        })
        .map_err(|_| "HTTP probe worker unavailable")?
        .join()
        .map_err(|_| "HTTP probe worker failed")??;
    Ok(result.into_value())
}

// System DNS is blocking. Bound caller waiting and cap even abandoned resolver
// workers; a timed-out worker can only return approved addresses, never connect.
static RESOLVERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct ResolverSlot;
impl Drop for ResolverSlot {
    fn drop(&mut self) {
        RESOLVERS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}
fn resolve_target(url: &str, expires: std::time::Instant) -> ProbeResult<std::net::SocketAddr> {
    use std::sync::atomic::Ordering;
    RESOLVERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 16).then_some(n + 1)
        })
        .map_err(|_| "HTTP probe resolver busy")?;
    let slot = ResolverSlot;
    let url = url.to_string();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    #[cfg(test)]
    let hook = BEFORE_RESOLVE.with(|hook| hook.borrow_mut().take());
    std::thread::Builder::new()
        .name("http-probe-resolve".into())
        .spawn(move || {
            let _slot = slot;
            #[cfg(test)]
            if let Some(hook) = hook {
                hook();
            }
            let target = super::validated_http_target(&url)
                .map_err(|e| super::format_ssrf_error(&e, false))
                .and_then(|target| {
                    target.ok_or_else(|| "probe_fetch requires SSRF protection".into())
                })
                .and_then(|target| {
                    target
                        .addresses
                        .first()
                        .copied()
                        .ok_or_else(|| "Could not resolve hostname".into())
                });
            let _ = sender.send(target);
        })
        .map_err(|_| "HTTP resolver worker unavailable")?;
    receiver
        .recv_timeout(expires.saturating_duration_since(std::time::Instant::now()))
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => "HTTP probe timeout",
            std::sync::mpsc::RecvTimeoutError::Disconnected => "HTTP resolver worker failed",
        })?
}

struct ProbeResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
    started: i64,
    finished: i64,
}

impl ProbeResponse {
    fn into_value(self) -> Value {
        Value::Map(HashMap::from([
            ("status".into(), Value::Int(self.status.into())),
            ("body".into(), Value::String(self.body)),
            (
                "headers".into(),
                Value::Map(
                    self.headers
                        .into_iter()
                        .map(|(k, v)| (k, Value::String(v)))
                        .collect(),
                ),
            ),
            ("sample_started_at_ms".into(), Value::Int(self.started)),
            ("sample_finished_at_ms".into(), Value::Int(self.finished)),
        ]))
    }
}

async fn exchange<S>(
    stream: S,
    request: Request<Body>,
    started: i64,
    expires: std::time::Instant,
) -> ProbeResult<ProbeResponse>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|_| "HTTP handshake failed")?;
    let driver = tokio::spawn(connection);
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| "HTTP request failed")?;
    let limit = super::max_response_size();
    if response
        .body()
        .size_hint()
        .upper()
        .is_some_and(|n| n > limit as u64)
    {
        return Err("HTTP response body too large".into());
    }
    let mut decode_headers = reqwest::header::HeaderMap::new();
    if let Some(value) = response.headers().get(hyper::header::CONTENT_TYPE) {
        if let Ok(value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
            decode_headers.insert(reqwest::header::CONTENT_TYPE, value);
        }
    }
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let mut incoming = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) =
        std::future::poll_fn(|cx| std::pin::Pin::new(&mut incoming).poll_frame(cx)).await
    {
        let frame = frame.map_err(|_| "HTTP response body failed")?;
        if let Ok(data) = frame.into_data() {
            if data.len() > limit.saturating_sub(bytes.len()) {
                return Err("HTTP response body too large".into());
            }
            bytes
                .try_reserve_exact(data.len())
                .map_err(|_| "HTTP response allocation failed")?;
            bytes.extend_from_slice(&data);
        }
    }
    driver.abort();
    let policy = super::RedirectPolicy {
        mode: super::RedirectMode::Manual,
        limit: 0,
        deadline: Some(expires),
    };
    let body = super::decode_http_text(&bytes, &decode_headers, policy, limit)
        .map_err(|_| "HTTP response decoding failed")?
        .map_err(|error| {
            if error == "HTTP chain timeout" {
                "HTTP probe timeout".into()
            } else {
                error
            }
        })?;
    Ok(ProbeResponse {
        status,
        headers,
        body,
        started,
        finished: chrono::Utc::now().timestamp_millis(),
    })
}

#[cfg(test)]
mod tests {
    use crate::interpreter::Value;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    fn call(opts: HashMap<String, Value>) -> std::result::Result<HashMap<String, Value>, String> {
        let module = super::super::init();
        let Some(Value::NativeFunction { func, .. }) = module.get("probe_fetch") else {
            panic!("std/http must export probe_fetch");
        };
        match func(&[Value::Map(opts)]).unwrap() {
            Value::EnumValue {
                variant,
                mut values,
                ..
            } if variant == "Ok" => {
                let Value::Map(map) = values.remove(0) else {
                    panic!("expected response map")
                };
                Ok(map)
            }
            Value::EnumValue {
                variant, values, ..
            } if variant == "Err" => {
                let Value::String(error) = &values[0] else {
                    panic!("expected error string")
                };
                Err(error.clone())
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }

    struct Fixture {
        url: String,
        accepts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        stop: std::sync::mpsc::Sender<()>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl Fixture {
        fn new(response: impl Fn(&mut std::net::TcpStream) + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/probe", listener.local_addr().unwrap());
            let accepts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed = accepts.clone();
            let (stop, done) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                while done.try_recv().is_err() {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            stream
                                .set_write_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            response(&mut stream);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(1))
                        }
                        Err(e) => panic!("{e}"),
                    }
                }
            });
            Self {
                url,
                accepts,
                stop,
                worker: Some(worker),
            }
        }
        fn opts(&self) -> HashMap<String, Value> {
            HashMap::from([
                ("url".into(), Value::String(self.url.clone())),
                ("timeout_ms".into(), Value::Int(2000)),
            ])
        }
        fn count(&self) -> usize {
            self.accepts.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            self.worker.take().unwrap().join().unwrap();
        }
    }
    fn respond(stream: &mut std::net::TcpStream) {
        let mut buf = [0; 4096];
        let _ = stream.read(&mut buf);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    }

    #[test]
    fn probe_worker_admission_is_bounded_and_released() {
        static TEST_PROBES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

        let slots: Vec<_> = (0..2)
            .map(|_| super::ProbeSlot::reserve_from(&TEST_PROBES, 2).unwrap())
            .collect();
        assert_eq!(
            super::ProbeSlot::reserve_from(&TEST_PROBES, 2).unwrap_err(),
            "HTTP probe worker busy"
        );
        drop(slots);
        assert!(super::ProbeSlot::reserve_from(&TEST_PROBES, 2).is_ok());
    }

    #[test]
    fn redirects_remain_manual_and_chunked_framing_is_decoded() {
        let target = Fixture::new(respond);
        let location = target.url.clone();
        let fixture = Fixture::new(move |stream| {
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            let _ = write!(stream, "HTTP/1.1 302 Found\r\nLocation: {location}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nhe\r\n3\r\nllo\r\n0\r\nX-Trailer: done\r\n\r\n");
        });
        for _ in 0..2 {
            let response = call(fixture.opts()).unwrap();
            assert!(matches!(response["status"], Value::Int(302)));
            assert!(matches!(&response["body"], Value::String(s) if s == "hello"));
        }
        assert_eq!(
            fixture.count(),
            2,
            "each call must use a fresh direct connection"
        );
        assert_eq!(target.count(), 0, "redirect must never be followed");
    }

    #[test]
    fn connection_close_and_invalid_framing_are_never_replayed() {
        for response in [
            "",
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\na",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nZ\r\nbad\r\n",
        ] {
            let fixture = Fixture::new(move |stream| {
                let mut buf = [0; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(response.as_bytes());
            });
            assert!(call(fixture.opts()).is_err());
            assert_eq!(
                fixture.count(),
                1,
                "a failed HTTP exchange must not be replayed"
            );
        }
    }

    #[test]
    fn blocked_resolution_returns_on_timeout_without_later_contact() {
        let fixture = Fixture::new(respond);
        let url = fixture.url.clone();
        let (release, paused) = std::sync::mpsc::channel();
        let (entered, ready) = std::sync::mpsc::channel();
        let (result, done) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            super::BEFORE_RESOLVE.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    entered.send(()).unwrap();
                    paused.recv().unwrap();
                }))
            });
            let outcome = call(HashMap::from([
                ("url".into(), Value::String(url)),
                ("timeout_ms".into(), Value::Int(30)),
            ]))
            .map(|_| ());
            result.send(outcome).unwrap();
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let outcome = done.recv_timeout(Duration::from_millis(250));
        release.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(outcome.unwrap(), Err("HTTP probe timeout".into()));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(fixture.count(), 0);
    }

    #[test]
    fn response_limit_rejects_declared_oversized_body() {
        let fixture = Fixture::new(|stream| {
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            let size = super::super::max_response_size().saturating_add(1);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n"
            );
        });
        assert_eq!(
            call(fixture.opts()).unwrap_err(),
            "HTTP response body too large"
        );
    }

    #[test]
    fn https_requires_verified_certificate_and_hostname() {
        for hostname in ["127.0.0.1", "wrong.invalid"] {
            let cert = rcgen::generate_simple_self_signed(vec![hostname.into()]).unwrap();
            let der = cert.cert.der().clone();
            let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![der.clone()],
                rustls_pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
            )
            .unwrap();
            let config = std::sync::Arc::new(config);
            let mut fixture = Fixture::new(move |stream| {
                let mut tls = rustls::StreamOwned::new(
                    rustls::ServerConnection::new(config.clone()).unwrap(),
                    stream,
                );
                let mut buf = [0; 4096];
                if tls.read(&mut buf).is_ok() {
                    let _ = tls.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                }
            });
            fixture.url = fixture.url.replacen("http:", "https:", 1);
            assert_eq!(
                call(fixture.opts()).unwrap_err(),
                "HTTP TLS verification or handshake failed"
            );
            super::TLS_ROOTS.with(|roots| roots.borrow_mut().push(der));
            let trusted = call(fixture.opts());
            if hostname == "127.0.0.1" {
                assert!(matches!(trusted.unwrap()["status"], Value::Int(200)));
            } else {
                assert_eq!(
                    trusted.unwrap_err(),
                    "HTTP TLS verification or handshake failed"
                );
            }
        }
    }

    #[test]
    fn options_fail_closed_before_target_contact() {
        let fixture = Fixture::new(respond);
        for (key, value) in [
            ("method", Value::String("POST".into())),
            ("method", Value::Int(1)),
            ("body", Value::String("not allowed".into())),
            ("redirect", Value::String("follow".into())),
            (
                "headers",
                Value::Map(HashMap::from([(
                    "X-Token".into(),
                    Value::Secret(
                        crate::interpreter::SecretValue::new("TEST", "canary-no-leak").unwrap(),
                    ),
                )])),
            ),
            ("timeout_ms", Value::Int(0)),
            ("timeout_ms", Value::Int(60_001)),
            ("timeout_ms", Value::String("100".into())),
            ("start_deadline_ms", Value::Int(-1)),
        ] {
            let mut opts = fixture.opts();
            opts.insert(key.into(), value);
            let error = call(opts).expect_err(key);
            assert!(!error.contains("canary-no-leak"));
        }
        assert_eq!(fixture.count(), 0);
        for url in [
            "http://169.254.169.254/",
            "http://10.0.0.1/",
            "file:///etc/passwd",
            "http://user:password@127.0.0.1/",
        ] {
            let error =
                call(HashMap::from([("url".into(), Value::String(url.into()))])).unwrap_err();
            assert!(!error.contains("password"));
        }
    }

    #[test]
    fn completion_timeout_is_independent_of_start_deadline() {
        let fixture = Fixture::new(|stream| {
            std::thread::sleep(Duration::from_millis(150));
            respond(stream);
        });
        let mut opts = fixture.opts();
        let deadline = chrono::Utc::now().timestamp_millis() + 100;
        opts.insert("start_deadline_ms".into(), Value::Int(deadline));
        let response = call(opts).unwrap();
        assert!(matches!(response["sample_started_at_ms"], Value::Int(n) if n < deadline));
        assert!(matches!(response["sample_finished_at_ms"], Value::Int(n) if n >= deadline));
        let mut opts = fixture.opts();
        opts.insert("timeout_ms".into(), Value::Int(30));
        assert_eq!(call(opts).unwrap_err(), "HTTP probe timeout");
    }

    #[test]
    fn delayed_socket_setup_expires_without_any_tcp_accept() {
        let fixture = Fixture::new(respond);
        for key in ["start_deadline_ms", "start_monotonic_deadline_ms"] {
            let now = if key == "start_deadline_ms" {
                chrono::Utc::now().timestamp_millis()
            } else {
                crate::stdlib::time::monotonic_millis()
            };
            let deadline = now + 40;
            let mut opts = fixture.opts();
            opts.insert(key.into(), Value::Int(deadline));
            super::BEFORE_CONNECT.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || loop {
                    let now = if key == "start_deadline_ms" {
                        chrono::Utc::now().timestamp_millis()
                    } else {
                        crate::stdlib::time::monotonic_millis()
                    };
                    if now >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }))
            });
            assert_eq!(call(opts).unwrap_err(), "start_deadline_expired");
        }
        assert_eq!(fixture.count(), 0, "expired probe made first contact");
    }

    #[test]
    fn probe_returns_loopback_response_with_contact_timestamps() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/probe?q=1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            assert!(
                request.starts_with("GET /probe?q=1 HTTP/1.1\r\n"),
                "{request}"
            );
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Probe: yes\r\nConnection: close\r\n\r\nhello").unwrap();
        });
        let before = chrono::Utc::now().timestamp_millis();
        let response = call(HashMap::from([
            ("url".into(), Value::String(url)),
            ("timeout_ms".into(), Value::Int(2000)),
        ]))
        .unwrap();
        assert!(matches!(response.get("status"), Some(Value::Int(200))));
        assert!(matches!(response.get("body"), Some(Value::String(s)) if s == "hello"));
        let Value::Map(headers) = &response["headers"] else {
            panic!("headers")
        };
        assert!(matches!(headers.get("x-probe"), Some(Value::String(s)) if s == "yes"));
        let Value::Int(start) = response["sample_started_at_ms"] else {
            panic!("start")
        };
        let Value::Int(finish) = response["sample_finished_at_ms"] else {
            panic!("finish")
        };
        assert!(
            before <= start && start <= finish && finish <= chrono::Utc::now().timestamp_millis()
        );
        server.join().unwrap();
    }
}
