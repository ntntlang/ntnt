#[path = "support/system_fixture.rs"]
mod fixture;
use ntnt::interpreter::{ExecutionMode, Interpreter, Value};
use ntnt::stdlib::net::tcp;
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
fn port(value: &Value) -> SocketAddr {
    let Value::Map(m) = tcp::local_addr(&[value.clone()]).unwrap() else {
        panic!()
    };
    SocketAddr::new(
        m["host"].to_string().parse().unwrap(),
        m["port"].to_string().parse().unwrap(),
    )
}
#[test]
fn tcp_bytes_eof_timeout_alias_and_independent_progress() {
    let _serial = TEST_LOCK.lock().unwrap();
    let listener = tcp::listen(&[Value::Int(0)]).unwrap();
    let addr = port(&listener);
    assert!(addr.ip().is_loopback());
    assert_ne!(addr.port(), 0);
    assert!(tcp::listen(&[Value::Int(i64::from(addr.port()))]).is_err());
    assert!(tcp::accept(&[listener.clone(), Value::Int(1)])
        .unwrap_err()
        .starts_with("timeout:"));
    let mut client = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    client.set_nonblocking(false).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let stream = tcp::accept(&[listener.clone(), Value::Int(100)]).unwrap();
    let alias = stream.clone();
    assert!(tcp::read(&[stream.clone(), Value::Int(100), Value::Int(1)])
        .unwrap_err()
        .starts_with("timeout:"));
    client.write_all(&[0, 255, 13, 10]).unwrap();
    let value = tcp::read(&[stream.clone(), Value::Int(2), Value::Int(100)]).unwrap();
    assert_eq!(value.to_string(), "[0, 255]");
    assert_eq!(
        tcp::read(&[stream.clone(), Value::Int(2), Value::Int(100)])
            .unwrap()
            .to_string(),
        "[13, 10]"
    );
    assert_eq!(
        tcp::write(&[stream.clone(), Value::String("reply\r\n".into())])
            .unwrap()
            .to_string(),
        "7"
    );
    let mut reply = [0; 7];
    client.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"reply\r\n");
    client.shutdown(std::net::Shutdown::Write).unwrap();
    for _ in 0..2 {
        assert_eq!(
            tcp::read(&[stream.clone(), Value::Int(1)])
                .unwrap()
                .to_string(),
            "none"
        );
    }
    tcp::close(&[listener.clone()]).unwrap();
    tcp::close(&[listener]).unwrap();
    assert!(tcp::peer_addr(&[stream.clone()]).is_ok());
    tcp::shutdown_stream(&[stream.clone(), Value::String("write".into())]).unwrap();
    assert!(tcp::write(&[stream.clone(), Value::String(String::new())]).is_err());
    tcp::close(&[stream]).unwrap();
    assert!(tcp::local_addr(&[alias.clone()])
        .unwrap_err()
        .starts_with("closed:"));
    tcp::close(&[alias]).unwrap();
}
#[test]
fn tcp_validation_capacity_rollbacks_and_last_drop() {
    let _serial = TEST_LOCK.lock().unwrap();
    for n in [-1, 65536, i64::MAX] {
        assert!(tcp::listen(&[Value::Int(n)]).is_err());
    }
    for value in [Value::String("localhost".into()), Value::Int(0)] {
        assert!(tcp::listen(&[
            Value::Int(0),
            Value::Map(std::collections::HashMap::from([("host".into(), value)]))
        ])
        .is_err());
    }
    let mut listeners = Vec::new();
    for _ in 0..128 {
        listeners.push(tcp::listen(&[Value::Int(0)]).unwrap());
    }
    assert!(tcp::listen(&[Value::Int(0)])
        .unwrap_err()
        .starts_with("capacity:"));
    let first = listeners.remove(0);
    let addr = port(&first);
    tcp::close(&[first]).unwrap();
    let replacement = tcp::listen(&[Value::Int(i64::from(addr.port()))]).unwrap();
    drop(replacement);
    drop(listeners);
    let fresh = tcp::listen(&[Value::Int(0)]).unwrap();
    let addr = port(&fresh);
    drop(fresh);
    let rebound = tcp::listen(&[Value::Int(i64::from(addr.port()))]).unwrap();
    drop(rebound);
    for _ in 0..256 {
        let listener = tcp::listen(&[Value::Int(0)]).unwrap();
        tcp::close(&[listener]).unwrap();
    }
}
#[test]
fn tcp_capability_denial_is_an_error_in_every_non_normal_mode() {
    let _serial = TEST_LOCK.lock().unwrap();
    let source = "import { tcp_listen } from \"std/net\"\nlet alias = tcp_listen\nalias(0)";
    let ast = ntnt::parser::Parser::new(ntnt::lexer::Lexer::new(source).collect())
        .parse()
        .unwrap();
    for mode in [
        ExecutionMode::Worker,
        ExecutionMode::HotReload,
        ExecutionMode::Job,
        ExecutionMode::UnitTest,
    ] {
        let mut interpreter = Interpreter::new();
        interpreter.set_execution_mode(mode);
        assert!(
            interpreter
                .eval(&ast)
                .unwrap_err()
                .to_string()
                .contains("capability:"),
            "{mode:?}"
        );
    }
}
#[test]
fn tcp_strict_types_lifecycle_and_json_rejection() {
    let _serial = TEST_LOCK.lock().unwrap();
    let output = fixture::strict_run(
        r#"
import { tcp_listen, tcp_accept, tcp_read, tcp_write, tcp_local_addr, tcp_peer_addr, tcp_shutdown, tcp_close } from "std/net"
fn open() -> Result<TcpListener, String> { tcp_listen(0) }
fn accept_one(l: TcpListener) -> Result<TcpStream, String> { tcp_accept(l, 1) }
fn read_one(s: TcpStream) -> Result<Array<Int>?, String> { tcp_read(s, 1, 1) }
fn write_one(s: TcpStream) -> Result<Int, String> { tcp_write(s, [0, 255], 1) }
fn peer(s: TcpStream) -> Result<Map<String, Any>, String> { tcp_peer_addr(s) }
fn finish(s: TcpStream) -> Result<Unit, String> { tcp_shutdown(s, "both") }
let l = unwrap(open())
let a = l
assert(a == l)
assert(typeof(l) == "TcpListener")
let addr = unwrap(tcp_local_addr(l))
assert(addr["host"] == "127.0.0.1")
assert(is_err(accept_one(l)))
unwrap(tcp_close(l))
unwrap(tcp_close(a))
print("TCP_SURFACE_OK")
"#,
    );
    assert!(output.contains("TCP_SURFACE_OK"));
    let mut child=fixture::Fixture::start("import { tcp_listen } from \"std/net\"\nimport { stringify } from \"std/json\"\nlet l = unwrap(tcp_listen(0))\nstringify(map { \"nested\": [l] })",&["run"],&[]);
    assert!(!child.wait().success());
    assert!(child.stderr().contains("TCP handles cannot"));
}
#[test]
fn native_multi_client_binary_fixture_makes_progress_with_idle_client() {
    let _serial = TEST_LOCK.lock().unwrap();
    let source = include_str!("../examples/system-primitives/smtp-fixture.tnt");
    let mut lint = fixture::Fixture::start(source, &["lint", "--strict"], &[]);
    assert!(lint.wait().success(), "{} {}", lint.stdout(), lint.stderr());
    let mut server = fixture::Fixture::start(source, &["run"], &[]);
    let addr = server.ready();
    let mut idle = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    idle.set_nonblocking(false).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut active = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    active.set_nonblocking(false).unwrap();
    active
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut greeting = [0; 11];
    idle.read_exact(&mut greeting).unwrap();
    assert_eq!(&greeting, b"220 local\r\n");
    active.read_exact(&mut greeting).unwrap();
    for chunk in [
        b"DATA\r".as_slice(),
        b"\n",
        &[0, 255, 13],
        &[10, 46, 46, 120, 13, 10, 46, 13],
        &[10],
    ] {
        active.write_all(chunk).unwrap();
    }
    let mut response = [0; 8];
    active
        .read_exact(&mut response)
        .unwrap_or_else(|e| panic!("{e}: {} {}", server.stdout(), server.stderr()));
    assert_eq!(&response, b"250 OK\r\n");
    let end = Instant::now() + Duration::from_secs(3);
    while !server.stdout().contains("CAPTURE [0,255,13,10,46,120]") {
        assert!(
            Instant::now() < end,
            "{} {}",
            server.stdout(),
            server.stderr()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(active);
    drop(idle);
}

#[test]
fn bound_without_listen_and_ipv6_literal_support() {
    let _serial = TEST_LOCK.lock().unwrap();
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    socket
        .bind(&"127.0.0.1:0".parse::<SocketAddr>().unwrap().into())
        .unwrap();
    let addr = socket.local_addr().unwrap().as_socket().unwrap();
    assert!(tcp::listen(&[Value::Int(i64::from(addr.port()))]).is_err());
    let options = Value::Map(std::collections::HashMap::from([(
        "host".into(),
        Value::String("::1".into()),
    )]));
    match tcp::listen(&[Value::Int(0), options]) {
        Ok(listener) => {
            assert!(port(&listener).is_ipv6());
            tcp::close(&[listener]).unwrap();
        }
        Err(e) => {
            assert!(
                std::net::TcpListener::bind("[::1]:0").is_err(),
                "IPv6 is available but native bind failed: {e}"
            );
            eprintln!("IPv6 loopback unavailable on this host: {e}");
        }
    }
}
#[test]
fn strict_lint_rejects_counterfeit_socket_types_and_wrong_arity() {
    let _serial = TEST_LOCK.lock().unwrap();
    for source in [
        "import { tcp_close } from \"std/net\"\nstruct TcpStream { n: Int }\ntcp_close(TcpStream { n: 1 })",
        "import { tcp_close } from \"std/net\"\ntcp_close(map { \"fd\": 3 })",
        "import { tcp_read } from \"std/net\"\ntcp_read(1)",
        "import { tcp_listen } from \"std/net\"\ntcp_listen(\"0\")",
    ] {
        let mut child=fixture::Fixture::start(source,&["lint","--strict"],&[]);assert!(!child.wait().success(),"accepted counterfeit: {source}");
    }
}
#[test]
fn stalled_reader_write_timeout_closes_stream_with_prefix_count() {
    let _serial = TEST_LOCK.lock().unwrap();
    let listener = tcp::listen(&[Value::Int(0)]).unwrap();
    let client = TcpStream::connect_timeout(&port(&listener), Duration::from_secs(2)).unwrap();
    socket2::SockRef::from(&client)
        .set_recv_buffer_size(1024)
        .unwrap();
    let stream = tcp::accept(&[listener.clone()]).unwrap();
    let data = Value::String("x".repeat(65536));
    let end = Instant::now() + Duration::from_secs(3);
    let mut failed = false;
    for _ in 0..512 {
        match tcp::write(&[stream.clone(), data.clone(), Value::Int(5)]) {
            Ok(_) => {}
            Err(e) => {
                assert!(
                    e.starts_with("write_failed: stream_closed=true; bytes_written="),
                    "{e}"
                );
                failed = true;
                break;
            }
        }
        assert!(
            Instant::now() < end,
            "write deadline failed to bound stalled client"
        );
    }
    assert!(failed, "stalled peer never filled finite kernel buffers");
    assert!(tcp::local_addr(&[stream])
        .unwrap_err()
        .starts_with("closed:"));
    tcp::close(&[listener]).unwrap();
}
