use ntnt::stdlib::net;
#[test]
fn persistent_api_is_registered() {
    let module = net::init();
    for name in ["ping_open", "ping_probe", "ping_close"] {
        assert!(module.contains_key(name), "missing std/net API: {name}");
    }
}

use ntnt::interpreter::{ExecutionMode, Interpreter, Value};
use ntnt::stdlib::net::persistent;
use std::collections::HashMap;
use std::time::{Duration, Instant};
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn eval(interp: &mut Interpreter, src: &str) -> ntnt::error::Result<Value> {
    let ast = ntnt::parser::Parser::new(ntnt::lexer::Lexer::new(src).collect())
        .parse()
        .unwrap();
    interp.eval(&ast)
}
fn options() -> Value {
    Value::Map(HashMap::from([
        ("allow_private".into(), Value::Bool(true)),
        ("idle_timeout_ms".into(), Value::Int(1000)),
    ]))
}
fn real_open(target: &str) -> Option<Value> {
    std::env::set_var("NTNT_NET_ALLOW_PRIVATE", "1");
    match persistent::open(&[Value::String(target.into()), options()]) {
        Ok(h) => Some(h),
        Err(e) => {
            assert!(e.starts_with("backend:"), "unexpected open error: {e}");
            assert_ne!(
                std::env::var("NTNT_ICMP_REQUIRE").as_deref(),
                Ok("1"),
                "required real ICMP setup failed for {target}: {e}"
            );
            eprintln!("SKIP real ICMP {target}: native socket unavailable: {e}; no actual probes asserted");
            None
        }
    }
}
fn result_map(v: Value) -> HashMap<String, Value> {
    let Value::Map(m) = v else { panic!("not map") };
    m
}
#[test]
fn invalid_arguments_policy_and_modes() {
    let _serial = SERIAL.lock().unwrap();
    for option in [
        Value::Int(0),
        Value::Int(-1),
        Value::Int(i64::MAX),
        Value::String("1000".into()),
    ] {
        let opts = Value::Map(HashMap::from([("idle_timeout_ms".into(), option)]));
        assert!(persistent::open(&[Value::String("127.0.0.1".into()), opts])
            .unwrap_err()
            .starts_with("invalid_argument:"));
    }
    for key in ["count", "interval_ms", "timeout_ms", "method", "unknown"] {
        let opts = Value::Map(HashMap::from([(key.into(), Value::Int(1))]));
        assert!(persistent::open(&[Value::String("127.0.0.1".into()), opts])
            .unwrap_err()
            .starts_with("invalid_argument:"));
    }
    assert!(persistent::open(&[Value::String("127.0.0.1".into())])
        .unwrap_err()
        .starts_with("policy:"));
    assert!(
        persistent::open(&[Value::String("169.254.169.254".into()), options()])
            .unwrap_err()
            .starts_with("policy:")
    );
    assert!(persistent::close(&[Value::Map(HashMap::new())])
        .unwrap_err()
        .starts_with("invalid_argument:"));
    let src = "import { ping_open } from \"std/net\"\nlet alias = ping_open\nalias(\"127.0.0.1\")";
    for mode in [ExecutionMode::HotReload, ExecutionMode::UnitTest] {
        let mut interp = Interpreter::new();
        interp.set_execution_mode(mode);
        assert!(eval(&mut interp, src)
            .unwrap_err()
            .to_string()
            .contains("capability:"));
    }
    for mode in [
        ExecutionMode::Normal,
        ExecutionMode::Worker,
        ExecutionMode::Job,
    ] {
        let mut interp = Interpreter::new();
        interp.set_execution_mode(mode);
        let result = eval(&mut interp, src).unwrap();
        assert!(result.to_string().contains("policy:"), "{mode:?}: {result}");
    }
}
#[test]
fn real_ipv4_repeated_independent_and_idle() {
    real_loopback("127.0.0.1");
}
#[test]
fn real_ipv6_repeated_independent_and_idle() {
    real_loopback("::1");
}
fn real_loopback(target: &str) {
    let _serial = SERIAL.lock().unwrap();
    let Some(h) = real_open(target) else {
        return;
    };
    let b = real_open(target).expect("second socket");
    let alias = h.clone();
    for seq in 1..=3 {
        let m = result_map(persistent::probe(&[h.clone(), Value::Int(500)]).unwrap());
        assert_eq!(m["status"].to_string(), "reply", "{m:?}");
        assert_eq!(m["seq"].to_string(), seq.to_string());
        assert_eq!(m["probe_id"].to_string(), seq.to_string());
        assert_eq!(m["target_addr"].to_string(), target);
        assert!(m.contains_key("latency_ms"));
    }
    assert_eq!(
        result_map(persistent::probe(std::slice::from_ref(&b)).unwrap())["seq"].to_string(),
        "1"
    );
    persistent::close(&[h]).unwrap();
    persistent::close(std::slice::from_ref(&alias)).unwrap();
    assert!(persistent::probe(&[alias])
        .unwrap_err()
        .starts_with("closed:"));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(persistent::probe(std::slice::from_ref(&b))
        .unwrap_err()
        .starts_with("expired:"));
    persistent::close(&[b]).unwrap();
    eprintln!("REAL ICMP {target}: repeated replies, independent sequence, alias close and idle expiry asserted");
}
#[test]
fn real_task_and_worker_interpreter() {
    let _serial = SERIAL.lock().unwrap();
    let Some(h) = real_open("127.0.0.1") else {
        return;
    };
    persistent::close(&[h]).unwrap();
    let mut interp = Interpreter::new();
    interp.set_execution_mode(ExecutionMode::Worker);
    eval(
        &mut interp,
        r#"
import { ping_open, ping_probe, ping_close } from "std/net"
fn open_local() -> Result<ProbeHandle, String> { ping_open("127.0.0.1", map { "allow_private": true }) }
fn sample(h: ProbeHandle) -> Result<Map<String, Any>, String> { ping_probe(h) }
let h = unwrap(open_local())
let alias = h
assert(h == alias)
assert(typeof(h) == "ProbeHandle")
let a = unwrap(sample(h))
let b = unwrap(sample(h))
assert(a.status == "reply")
assert(b.seq == 2)
unwrap(ping_close(h))
"#,
    )
    .unwrap();
    let mut interp = Interpreter::new();
    eval(
        &mut interp,
        r#"
import { ping_open, ping_probe, ping_close } from "std/net"
import { spawn, await_task } from "std/concurrent"
let task = spawn(fn() {
    let h = unwrap(ping_open("127.0.0.1", map { "allow_private": true }))
    let a = unwrap(ping_probe(h))
    let b = unwrap(ping_probe(h))
    unwrap(ping_close(h))
    a.status == "reply" && b.seq == 2
})
assert(unwrap(await_task(task)))
"#,
    )
    .unwrap();
    eprintln!("REAL ICMP: Worker-mode interpreter and spawned task sent repeated probes");
}
#[test]
fn real_controlled_timeout_fixture() {
    let _serial = SERIAL.lock().unwrap();
    let Ok(target) = std::env::var("NTNT_ICMP_TIMEOUT_TARGET") else {
        eprintln!("SKIP controlled real timeout: set NTNT_ICMP_TIMEOUT_TARGET to an approved target whose echo replies are dropped");
        return;
    };
    let Some(h) = real_open(&target) else {
        return;
    };
    let start = Instant::now();
    let m = result_map(persistent::probe(&[h.clone(), Value::Int(200)]).unwrap());
    assert_eq!(
        m["status"].to_string(),
        "timeout",
        "fixture must DROP, not REJECT: {m:?}"
    );
    assert_eq!(m["probe_id"].to_string(), "1");
    assert!(!m.contains_key("latency_ms"));
    assert!(start.elapsed() >= Duration::from_millis(190));
    assert!(start.elapsed() < Duration::from_secs(1));
    persistent::close(&[h]).unwrap();
    let Value::ProbeHandle(owner) = real_open(&target).unwrap() else {
        panic!()
    };
    let active = owner.clone();
    let thread = std::thread::spawn(move || {
        persistent::probe(&[Value::ProbeHandle(active), Value::Int(30000)]).map(|_| ())
    });
    std::thread::sleep(Duration::from_millis(50));
    let start = Instant::now();
    persistent::close(&[Value::ProbeHandle(owner)]).unwrap();
    assert!(start.elapsed() < Duration::from_millis(50));
    assert!(thread
        .join()
        .unwrap()
        .unwrap_err()
        .starts_with("cancelled:"));
    eprintln!("REAL ICMP controlled timeout and active cancellation asserted for {target}");
}
#[test]
fn real_permission_denial_fixture() {
    let _serial = SERIAL.lock().unwrap();
    if std::env::var("NTNT_ICMP_EXPECT_DENIED").as_deref() != Ok("1") {
        eprintln!("SKIP permission denial assertion: run with both dgram and raw ICMP denied and NTNT_ICMP_EXPECT_DENIED=1");
        return;
    }
    std::env::set_var("NTNT_NET_ALLOW_PRIVATE", "1");
    for target in ["127.0.0.1", "::1"] {
        let error = persistent::open(&[Value::String(target.into()), options()]).unwrap_err();
        assert!(error.starts_with("backend:"), "{error}");
        assert!(
            error.contains("permitted") || error.contains("denied"),
            "expected permission failure, got {error}"
        );
    }
    eprintln!("REAL native socket permission denial asserted (no probes sent)");
}

#[path = "support/system_fixture.rs"]
mod fixture;
#[test]
fn typed_surface_and_complete_example_lint() {
    let mut example = fixture::Fixture::start(
        include_str!("../examples/persistent-icmp.tnt"),
        &["lint", "--strict"],
        &[],
    );
    assert!(
        example.wait().success(),
        "{} {}",
        example.stdout(),
        example.stderr()
    );
    let source = r#"
import { ping_open, ping_probe, ping_close } from "std/net"
fn open(target: String) -> Result<ProbeHandle, String> { ping_open(target) }
fn one(h: ProbeHandle) -> Result<Map<String, Any>, String> { ping_probe(h, 1000) }
fn finish(h: ProbeHandle) -> Result<Unit, String> { ping_close(h) }
"#;
    let mut child = fixture::Fixture::start(source, &["lint", "--strict"], &[]);
    assert!(
        child.wait().success(),
        "{} {}",
        child.stdout(),
        child.stderr()
    );
}
#[test]
fn real_http_worker_owns_persistent_socket() {
    let _serial = SERIAL.lock().unwrap();
    let Some(h) = real_open("127.0.0.1") else {
        return;
    };
    persistent::close(&[h]).unwrap();
    let source = r#"
import { ping_open, ping_probe } from "std/net"
import { json } from "std/http/server"
let probe = unwrap(ping_open("127.0.0.1", map { "allow_private": true }))
get("/probe", fn(req) {
    let first = unwrap(ping_probe(probe))
    let second = unwrap(ping_probe(probe))
    json(map { "first": first, "second": second })
})
listen(0, map { "fixture": true, "readiness": "json" })
"#;
    let mut server = fixture::Fixture::start(
        source,
        &["run"],
        &[
            ("NTNT_ENV", "production"),
            ("NTNT_WORKERS", "2"),
            ("NTNT_NET_ALLOW_PRIVATE", "1"),
        ],
    );
    let addr = server.ready();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    for _ in 0..3 {
        let response: serde_json::Value = client
            .get(format!("http://{addr}/probe"))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(response["first"]["status"], "reply");
        assert_eq!(response["second"]["status"], "reply");
        assert_eq!(
            response["second"]["seq"].as_i64().unwrap(),
            response["first"]["seq"].as_i64().unwrap() + 1
        );
    }
    eprintln!("REAL ICMP: actual two-worker HTTP runtime returned repeated probes on worker-owned sessions");
}
