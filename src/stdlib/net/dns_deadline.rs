//! Per-lookup first-contact guard using Hickory's public runtime extension point.
//! Deadlines authorize userspace UDP send / TCP connect, not NIC wire time.
//! Once contact is possible, normal replies, retries and fallback may finish.
use super::*;
use crate::stdlib::send_deadline::SendDeadline;
use hickory_resolver::{
    name_server::{GenericConnector, RuntimeProvider},
    proto::{iocompat::AsyncIoTokioAsStd, udp::DnsUdpSocket, TokioTime},
    AsyncResolver, TokioHandle,
};
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{ready, Context, Poll},
};

#[derive(Default)]
struct ContactState {
    started: bool,
    denied: bool,
}

#[derive(Clone)]
struct Guard {
    deadline: SendDeadline,
    state: Arc<Mutex<ContactState>>,
}

impl Guard {
    fn new(deadline: SendDeadline) -> Self {
        Self {
            deadline,
            state: Arc::default(),
        }
    }

    fn authorize(&self, state: &mut ContactState) -> io::Result<()> {
        if !state.started && (state.denied || self.deadline.check().is_err()) {
            state.denied = true;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "start_deadline_expired",
            ));
        }
        Ok(())
    }

    fn send_udp(&self, send: impl FnOnce() -> io::Result<usize>) -> io::Result<usize> {
        let mut state = self.state.lock().unwrap();
        self.authorize(&mut state)?;
        // No await/setup between authorization and the nonblocking syscall.
        let result = send();
        // WouldBlock proves nothing was sent. All other errors are conservatively
        // ambiguous: never relabel possible contact as a later no-send expiry.
        if !matches!(&result, Err(e) if e.kind() == io::ErrorKind::WouldBlock) {
            state.started = true;
        }
        result
    }

    fn connect_tcp(&self, socket: &socket2::Socket, addr: &socket2::SockAddr) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        self.authorize(&mut state)?;
        let result = socket.connect(addr);
        // Connect initiation (including EINPROGRESS or an ambiguous error) can
        // emit SYN traffic. DNS framing writes are NOT the first-contact boundary.
        state.started = true;
        result
    }

    fn denied_without_contact(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.denied && !state.started
    }
}

// Test-only pauses sit AFTER resolver/socket setup and readiness, not at entry.
#[derive(Clone, Default)]
struct Pauses {
    #[cfg(test)]
    setup: Duration,
    #[cfg(test)]
    bind: Duration,
    #[cfg(test)]
    udp: Duration,
    #[cfg(test)]
    connect: Duration,
}

struct GuardedUdp {
    socket: tokio::net::UdpSocket,
    guard: Guard,
    #[cfg(test)]
    pause: Duration,
}

impl DnsUdpSocket for GuardedUdp {
    type Time = TokioTime;

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        <tokio::net::UdpSocket as DnsUdpSocket>::poll_recv_from(&self.socket, cx, buf)
    }

    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        loop {
            // Pending does not commit the window. A wake or WouldBlock retry
            // always passes through a fresh check next to try_send_to.
            ready!(self.socket.poll_send_ready(cx))?;
            #[cfg(test)]
            std::thread::sleep(self.pause);
            match self.guard.send_udp(|| self.socket.try_send_to(buf, target)) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                result => return Poll::Ready(result),
            }
        }
    }
}

#[derive(Clone)]
struct Provider {
    handle: TokioHandle,
    guard: Guard,
    #[cfg(test)]
    pauses: Pauses,
}

impl RuntimeProvider for Provider {
    type Handle = TokioHandle;
    type Timer = TokioTime;
    type Udp = GuardedUdp;
    type Tcp = AsyncIoTokioAsStd<tokio::net::TcpStream>;

    fn create_handle(&self) -> Self::Handle {
        self.handle.clone()
    }

    fn bind_udp(
        &self,
        local: SocketAddr,
        _server: SocketAddr,
    ) -> Pin<Box<dyn Send + Future<Output = io::Result<Self::Udp>>>> {
        let provider = self.clone();
        Box::pin(async move {
            // Preserve Hickory's randomized local address/port and unconnected
            // socket; its normal source/ID validation remains untouched.
            let socket = tokio::net::UdpSocket::bind(local).await?;
            #[cfg(test)]
            std::thread::sleep(provider.pauses.bind);
            Ok(GuardedUdp {
                socket,
                guard: provider.guard,
                #[cfg(test)]
                pause: provider.pauses.udp,
            })
        })
    }

    fn connect_tcp(
        &self,
        server: SocketAddr,
    ) -> Pin<Box<dyn Send + Future<Output = io::Result<Self::Tcp>>>> {
        let provider = self.clone();
        Box::pin(async move {
            let socket = socket2::Socket::new(
                socket2::Domain::for_address(server),
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;
            socket.set_nonblocking(true)?;
            let addr = server.into();
            #[cfg(test)]
            std::thread::sleep(provider.pauses.connect);
            if let Err(error) = provider.guard.connect_tcp(&socket, &addr) {
                let in_progress = error.kind() == io::ErrorKind::WouldBlock;
                #[cfg(unix)]
                let in_progress = in_progress || error.raw_os_error() == Some(libc::EINPROGRESS);
                if !in_progress {
                    return Err(error);
                }
            }
            let stream = tokio::net::TcpStream::from_std(socket.into())?;
            stream.writable().await?;
            if let Some(error) = stream.take_error()? {
                return Err(error);
            }
            Ok(AsyncIoTokioAsStd(stream))
        })
    }
}

pub(super) fn lookup(
    name: &str,
    kind: DnsRecordType,
    opts: Option<&HashMap<String, Value>>,
    deadline: SendDeadline,
) -> Result<Vec<DnsAnswer>, String> {
    let (config, resolver_opts) = super::dns_resolver_options(opts)?;
    lookup_with_config(
        config,
        resolver_opts,
        deadline,
        name,
        kind,
        Pauses::default(),
    )
}

fn lookup_with_config(
    config: ResolverConfig,
    opts: ResolverOpts,
    deadline: SendDeadline,
    name: &str,
    kind: DnsRecordType,
    _pauses: Pauses,
) -> Result<Vec<DnsAnswer>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to initialize DNS resolver: {}", e))?;
    let guard = Guard::new(deadline);
    let provider = Provider {
        handle: TokioHandle::default(),
        guard: guard.clone(),
        #[cfg(test)]
        pauses: _pauses.clone(),
    };
    let resolver = AsyncResolver::new(config, opts, GenericConnector::new(provider));
    #[cfg(test)]
    std::thread::sleep(_pauses.setup);
    let result = runtime.block_on(resolver.lookup(name, kind.hickory_type()));
    // Hickory can aggregate/drop I/O error text. Consult per-operation state
    // before its normal no-record translation, without rejecting local answers.
    if result.is_err() && guard.denied_without_contact() {
        return Err("start_deadline_expired".into());
    }
    super::dns_lookup_result(result, name, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::{
        config::{NameServerConfig, Protocol},
        proto::{
            op::{Message, MessageType},
            rr::{rdata::A, RData, Record},
        },
    };
    use std::{
        io::{Read, Write},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
    };
    fn answer(query: &[u8], truncated: bool) -> Vec<u8> {
        let q = Message::from_vec(query).unwrap();
        let mut r = Message::new();
        r.set_id(q.id())
            .set_message_type(MessageType::Response)
            .set_recursion_desired(true)
            .set_recursion_available(true)
            .set_truncated(truncated);
        for query in q.queries() {
            r.add_query(query.clone());
            if !truncated {
                r.add_answer(Record::from_rdata(
                    query.name().clone(),
                    60,
                    RData::A(A::new(192, 0, 2, 7)),
                ));
            }
        }
        r.to_vec().unwrap()
    }
    struct Fixture {
        addr: SocketAddr,
        udp: Arc<AtomicUsize>,
        tcp: Arc<AtomicUsize>,
        tcp_bytes: Arc<AtomicUsize>,
        connections: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        threads: Vec<thread::JoinHandle<()>>,
    }
    impl Fixture {
        fn new(truncated: bool, delay_ms: u64, drop_first: bool) -> Self {
            let tcp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = tcp_listener.local_addr().unwrap();
            tcp_listener.set_nonblocking(true).unwrap();
            let udp_socket = std::net::UdpSocket::bind(addr).unwrap();
            udp_socket
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            let udp = Arc::new(AtomicUsize::new(0));
            let tcp = Arc::new(AtomicUsize::new(0));
            let connections = Arc::new(AtomicUsize::new(0));
            let tcp_bytes = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let (u, s) = (udp.clone(), stop.clone());
            let ut = thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while !s.load(Ordering::SeqCst) {
                    if let Ok((n, src)) = udp_socket.recv_from(&mut buf) {
                        assert!(src.ip().is_loopback());
                        let count = u.fetch_add(1, Ordering::SeqCst) + 1;
                        if drop_first && count == 1 {
                            continue;
                        }
                        thread::sleep(Duration::from_millis(delay_ms));
                        udp_socket
                            .send_to(&answer(&buf[..n], truncated), src)
                            .unwrap();
                    }
                }
            });
            let (t, b, c, s) = (
                tcp.clone(),
                tcp_bytes.clone(),
                connections.clone(),
                stop.clone(),
            );
            let tt = thread::spawn(move || {
                while !s.load(Ordering::SeqCst) {
                    match tcp_listener.accept() {
                        Ok((mut stream, src)) => {
                            assert!(src.ip().is_loopback());
                            c.fetch_add(1, Ordering::SeqCst);
                            stream
                                .set_read_timeout(Some(Duration::from_millis(500)))
                                .unwrap();
                            let mut len = [0; 2];
                            match stream.read(&mut len) {
                                Ok(n) if n > 0 => {
                                    b.fetch_add(n, Ordering::SeqCst);
                                    if n < 2 {
                                        if stream.read_exact(&mut len[n..]).is_err() {
                                            continue;
                                        }
                                    }
                                }
                                _ => continue,
                            };
                            let n = u16::from_be_bytes(len) as usize;
                            let mut body = vec![0; n];
                            if stream.read_exact(&mut body).is_ok() {
                                b.fetch_add(n, Ordering::SeqCst);
                                t.fetch_add(1, Ordering::SeqCst);
                                thread::sleep(Duration::from_millis(delay_ms));
                                let resp = answer(&body, false);
                                let _ = stream.write_all(&(resp.len() as u16).to_be_bytes());
                                let _ = stream.write_all(&resp);
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2))
                        }
                        Err(e) => panic!("{e}"),
                    }
                }
            });
            Self {
                addr,
                udp,
                tcp,
                tcp_bytes,
                connections,
                stop,
                threads: vec![ut, tt],
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            for t in self.threads.drain(..) {
                t.join().unwrap();
            }
        }
    }

    fn fixture_options(
        fixture: &Fixture,
        tcp_only: bool,
        retry: bool,
    ) -> (ResolverConfig, ResolverOpts) {
        let mut config = ResolverConfig::new();
        if !tcp_only {
            config.add_name_server(NameServerConfig::new(fixture.addr, Protocol::Udp));
        }
        config.add_name_server(NameServerConfig::new(fixture.addr, Protocol::Tcp));
        let mut opts = ResolverOpts::default();
        opts.timeout = Duration::from_millis(if retry { 150 } else { 800 });
        opts.attempts = 1;
        opts.num_concurrent_reqs = 1;
        opts.use_hosts_file = false;
        (config, opts)
    }

    fn deadline(ms: i64) -> SendDeadline {
        SendDeadline::parse(Some(&HashMap::from([
            (
                "start_deadline_ms".into(),
                Value::Int(chrono::Utc::now().timestamp_millis() + ms),
            ),
            (
                "start_monotonic_deadline_ms".into(),
                Value::Int(crate::stdlib::time::monotonic_millis() + ms),
            ),
        ])))
        .unwrap()
    }

    #[test]
    fn dns_deadline_delayed_setup_has_zero_udp_and_tcp_contact() {
        for tcp_only in [false, true] {
            let fixture = Fixture::new(false, 0, false);
            let (config, opts) = fixture_options(&fixture, tcp_only, false);
            let result = lookup_with_config(
                config,
                opts,
                deadline(50),
                "fixture.example.",
                DnsRecordType::parse("A").unwrap(),
                Pauses {
                    setup: Duration::from_millis(100),
                    ..Pauses::default()
                },
            );
            thread::sleep(Duration::from_millis(25));
            assert_eq!(fixture.udp.load(Ordering::SeqCst), 0, "late UDP contact");
            assert_eq!(
                fixture.connections.load(Ordering::SeqCst),
                0,
                "late TCP connect"
            );
            assert_eq!(result.unwrap_err(), "start_deadline_expired");
        }
    }

    #[test]
    fn dns_deadline_expiry_after_bind_readiness_and_tcp_setup_has_no_contact() {
        for (tcp_only, pauses) in [
            (
                false,
                Pauses {
                    bind: Duration::from_millis(100),
                    ..Pauses::default()
                },
            ),
            (
                false,
                Pauses {
                    udp: Duration::from_millis(100),
                    ..Pauses::default()
                },
            ),
            (
                true,
                Pauses {
                    connect: Duration::from_millis(100),
                    ..Pauses::default()
                },
            ),
        ] {
            let fixture = Fixture::new(false, 0, false);
            let (config, opts) = fixture_options(&fixture, tcp_only, false);
            let result = lookup_with_config(
                config,
                opts,
                deadline(50),
                "fixture.example.",
                DnsRecordType::parse("A").unwrap(),
                pauses,
            );
            thread::sleep(Duration::from_millis(25));
            assert_eq!(fixture.udp.load(Ordering::SeqCst), 0);
            assert_eq!(fixture.connections.load(Ordering::SeqCst), 0);
            assert_eq!(fixture.tcp_bytes.load(Ordering::SeqCst), 0);
            assert_eq!(result.unwrap_err(), "start_deadline_expired");
        }
    }

    #[test]
    fn dns_deadline_allows_late_reply_tcp_fallback_and_retry() {
        for (tcp_only, truncated, reply_delay, drop_first, expected_udp, expected_tcp) in [
            (false, false, 180, false, 1, 0),
            (false, true, 180, false, 1, 1),
            (true, false, 180, false, 0, 1),
            (false, false, 0, true, 2, 0),
        ] {
            let fixture = Fixture::new(truncated, reply_delay, drop_first);
            let (config, opts) = fixture_options(&fixture, tcp_only, drop_first);
            let result = lookup_with_config(
                config,
                opts,
                deadline(100),
                "fixture.example.",
                DnsRecordType::parse("A").unwrap(),
                Pauses::default(),
            )
            .unwrap();
            assert_eq!(result[0].value, "192.0.2.7");
            assert_eq!(result[0].record_type, "A");
            assert_eq!(result[0].ttl, 60);
            assert_eq!(fixture.udp.load(Ordering::SeqCst), expected_udp);
            assert_eq!(fixture.tcp.load(Ordering::SeqCst), expected_tcp);
            assert_eq!(fixture.connections.load(Ordering::SeqCst), expected_tcp);
        }
    }

    #[test]
    fn dns_deadline_pending_readiness_does_not_latch() {
        use std::task::{Wake, Waker};
        struct Noop;
        impl Wake for Noop {
            fn wake(self: Arc<Self>) {}
        }
        let fixture = Fixture::new(false, 0, false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _entered = runtime.enter();
        let guard = Guard::new(deadline(50));
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let socket = GuardedUdp {
            socket: tokio::net::UdpSocket::from_std(socket).unwrap(),
            guard: guard.clone(),
            pause: Duration::ZERO,
        };
        let waker = Waker::from(Arc::new(Noop));
        let mut cx = Context::from_waker(&waker);
        assert!(socket
            .poll_send_to(&mut cx, b"query", fixture.addr)
            .is_pending());
        assert!(!guard.state.lock().unwrap().started);
        thread::sleep(Duration::from_millis(100));
        let result = runtime.block_on(std::future::poll_fn(|cx| {
            socket.poll_send_to(cx, b"query", fixture.addr)
        }));
        assert_eq!(result.unwrap_err().to_string(), "start_deadline_expired");
        assert_eq!(fixture.udp.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn dns_deadline_would_block_rechecks_but_ambiguous_error_commits() {
        let guard = Guard::new(deadline(50));
        assert_eq!(
            guard
                .send_udp(|| Err(io::ErrorKind::WouldBlock.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(!guard.state.lock().unwrap().started);
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            guard
                .send_udp(|| panic!("expired retry sent"))
                .unwrap_err()
                .to_string(),
            "start_deadline_expired"
        );
        assert!(guard.denied_without_contact());

        let guard = Guard::new(deadline(50));
        assert!(guard
            .send_udp(|| Err(io::ErrorKind::ConnectionRefused.into()))
            .is_err());
        thread::sleep(Duration::from_millis(100));
        assert_eq!(guard.send_udp(|| Ok(1)).unwrap(), 1);
        assert!(!guard.denied_without_contact());
    }

    #[test]
    fn dns_deadline_tcp_initiation_commits_before_dns_write() {
        let fixture = Fixture::new(false, 0, false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let guard = Guard::new(deadline(100));
        let provider = Provider {
            handle: TokioHandle::default(),
            guard: guard.clone(),
            pauses: Pauses::default(),
        };
        let stream = runtime
            .block_on(provider.connect_tcp(fixture.addr))
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        assert!(guard.state.lock().unwrap().started);
        assert!(!guard.denied_without_contact());
        assert_eq!(fixture.connections.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.tcp_bytes.load(Ordering::SeqCst), 0);
        assert_eq!(guard.send_udp(|| Ok(1)).unwrap(), 1);
        drop(stream);
    }

    #[test]
    fn dns_deadline_either_clock_denies_but_local_and_cached_answers_work() {
        let fixture = Fixture::new(false, 0, false);
        let (config, opts) = fixture_options(&fixture, false, false);
        for key in ["start_deadline_ms", "start_monotonic_deadline_ms"] {
            let mut values = HashMap::from([
                ("start_deadline_ms".into(), Value::Int(i64::MAX)),
                ("start_monotonic_deadline_ms".into(), Value::Int(i64::MAX)),
            ]);
            values.insert(key.into(), Value::Int(0));
            let deadline = SendDeadline::parse(Some(&values)).unwrap();
            assert_eq!(
                lookup_with_config(
                    config.clone(),
                    opts.clone(),
                    deadline.clone(),
                    "uncached.example.",
                    DnsRecordType::parse("A").unwrap(),
                    Pauses::default()
                )
                .unwrap_err(),
                "start_deadline_expired"
            );
            let local = lookup_with_config(
                config.clone(),
                opts.clone(),
                deadline,
                "localhost.",
                DnsRecordType::parse("A").unwrap(),
                Pauses::default(),
            )
            .unwrap();
            assert_eq!(local[0].value, "127.0.0.1");
        }
        assert_eq!(fixture.udp.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.connections.load(Ordering::SeqCst), 0);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let guard = Guard::new(deadline(100));
        let provider = Provider {
            handle: TokioHandle::default(),
            guard: guard.clone(),
            pauses: Pauses::default(),
        };
        let resolver = AsyncResolver::new(config, opts, GenericConnector::new(provider));
        let first = runtime
            .block_on(resolver.lookup("cached.example.", RecordType::A))
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        // Fixture-only fresh operation state against a populated Hickory cache.
        *guard.state.lock().unwrap() = ContactState::default();
        let cached = runtime
            .block_on(resolver.lookup("cached.example.", RecordType::A))
            .unwrap();
        assert_eq!(first.iter().next(), cached.iter().next());
        assert_eq!(fixture.udp.load(Ordering::SeqCst), 1);
        assert!(!guard.state.lock().unwrap().started);
        assert!(!guard.denied_without_contact());
    }

    #[test]
    fn dns_deadline_retries_and_none_options_are_compatible() {
        assert!(!SendDeadline::parse(None).unwrap().is_configured());
        assert!(!SendDeadline::parse(Some(&HashMap::from([
            ("start_deadline_ms".into(), Value::none()),
            ("start_monotonic_deadline_ms".into(), Value::none()),
        ])))
        .unwrap()
        .is_configured());
        assert_eq!(dns_resolver_options(None).unwrap().1.attempts, 1);
        for retries in [0, 1, 2] {
            assert_eq!(
                dns_resolver_options(Some(&HashMap::from([(
                    "retries".into(),
                    Value::Int(retries)
                )])))
                .unwrap()
                .1
                .attempts,
                retries as usize
            );
        }
        for value in [
            Value::Int(-1),
            Value::Int(3),
            Value::Bool(true),
            Value::Float(1.0),
            Value::String("0".into()),
        ] {
            for guarded in [false, true] {
                let mut opts = HashMap::from([("retries".into(), value.clone())]);
                if guarded {
                    opts.insert("start_deadline_ms".into(), Value::Int(i64::MAX));
                }
                let result = dns_lookup_fn(&[
                    Value::String("localhost.".into()),
                    Value::String("A".into()),
                    Value::Map(opts),
                ])
                .unwrap();
                assert!(matches!(result, Value::EnumValue {variant, ..} if variant == "Err"));
            }
        }
    }

    #[test]
    fn dns_deadline_retries_zero_sends_once_on_loss() {
        let fixture = Fixture::new(false, 0, true);
        let (config, mut opts) = fixture_options(&fixture, false, true);
        opts.attempts =
            dns_resolver_options(Some(&HashMap::from([("retries".into(), Value::Int(0))])))
                .unwrap()
                .1
                .attempts;
        let result = lookup_with_config(
            config,
            opts,
            deadline(100),
            "fixture.example.",
            DnsRecordType::parse("A").unwrap(),
            Pauses::default(),
        );
        assert_eq!(fixture.udp.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.connections.load(Ordering::SeqCst), 0);
        assert!(result.is_err());
        assert_ne!(result.unwrap_err(), "start_deadline_expired");
    }

    #[test]
    fn dns_deadline_public_local_answers_preserve_record_maps() {
        for value in [None, Some(Value::none()), Some(Value::Int(0))] {
            let mut opts = HashMap::new();
            if let Some(value) = value {
                opts.insert("start_deadline_ms".into(), value);
            }
            let result = dns_lookup_fn(&[
                Value::String("localhost.".into()),
                Value::String("A".into()),
                Value::Map(opts),
            ])
            .unwrap();
            let Value::EnumValue {
                variant, values, ..
            } = result
            else {
                panic!("not Result")
            };
            assert_eq!(variant, "Ok");
            let Value::Array(answers) = &values[0] else {
                panic!("not Array")
            };
            let Value::Map(answer) = &answers[0] else {
                panic!("not Map")
            };
            assert_eq!(answer.len(), 4);
            assert!(matches!(&answer["type"], Value::String(s) if s == "A"));
            assert!(matches!(&answer["value"], Value::String(s) if s == "127.0.0.1"));
            assert!(matches!(&answer["name"], Value::String(s) if s == "localhost."));
            assert!(matches!(&answer["ttl"], Value::Int(n) if *n > 0));
        }
    }

    #[test]
    fn dns_deadline_rejects_malformed_both_clocks() {
        for key in ["start_deadline_ms", "start_monotonic_deadline_ms"] {
            for value in [
                Value::Int(-1),
                Value::String("1000".into()),
                Value::Float(1000.0),
                Value::Bool(true),
                Value::Unit,
            ] {
                let result = dns_lookup_fn(&[
                    Value::String("localhost.".into()),
                    Value::String("A".into()),
                    Value::Map(HashMap::from([(key.into(), value)])),
                ])
                .unwrap();
                assert!(
                    matches!(&result, Value::EnumValue { variant, values, .. }
                    if variant == "Err" && matches!(&values[0], Value::String(s) if s == &format!("{key} must be a nonnegative Int or None"))),
                    "malformed {key} accepted: {result:?}"
                );
            }
        }
    }
}
