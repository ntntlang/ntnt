use crate::interpreter::{SecretValue, Value};
use crate::stdlib::json::{intent_value_to_json, intent_value_to_json_reject};
use crate::stdlib::secrets;
use base64::Engine;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const SECRET_CANARY: &str = "s3cr3t-canary-value";

#[test]
fn secret_value_is_redacted_in_display_and_debug_output() {
    let secret = SecretValue::new("TEST_SECRET", SECRET_CANARY).expect("valid secret");
    let value = Value::Secret(secret);

    let display = value.to_string();
    let debug = format!("{value:?}");

    assert_eq!(display, "[REDACTED]");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(SECRET_CANARY));
}

#[test]
fn secret_value_has_distinct_runtime_type_and_is_truthy() {
    let value =
        Value::Secret(SecretValue::new("TEST_SECRET", SECRET_CANARY).expect("valid secret"));

    assert_eq!(value.type_name(), "Secret");
    assert!(value.is_truthy());
}

#[test]
fn contains_secret_finds_nested_secret_values() {
    let secret =
        Value::Secret(SecretValue::new("TEST_SECRET", SECRET_CANARY).expect("valid secret"));
    let mut nested = HashMap::new();
    nested.insert(
        "payload".to_string(),
        Value::Array(vec![Value::some(secret)]),
    );

    assert!(Value::Map(nested).contains_secret());
    assert!(!Value::Array(vec![Value::String("safe".to_string())]).contains_secret());
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("environment test lock")
}

fn native_fn(module: &HashMap<String, Value>, name: &str) -> fn(&[Value]) -> crate::Result<Value> {
    match module.get(name) {
        Some(Value::NativeFunction { func, .. }) => *func,
        other => panic!("expected native function {name}, got {other:?}"),
    }
}

#[test]
fn secret_names_reject_unsafe_diagnostic_content() {
    for invalid in ["", "1STARTS_WITH_DIGIT", "HAS SPACE", "HAS\nNEWLINE"] {
        let err = SecretValue::new(invalid, SECRET_CANARY).expect_err("name must be rejected");
        assert!(!err.to_string().contains(SECRET_CANARY));
    }

    let too_long = "A".repeat(129);
    SecretValue::new(too_long, SECRET_CANARY).expect_err("overlong name must be rejected");
}

#[test]
fn std_secrets_gets_an_optional_redacted_secret_from_env() {
    let _guard = env_lock();
    let name = "NTNT_TEST_STD_SECRET_OPTIONAL";
    std::env::remove_var("NTNT_SECRETS_PROVIDER");
    std::env::set_var(name, SECRET_CANARY);

    let module = secrets::init();
    let value = native_fn(&module, "get_secret")(&[Value::String(name.to_string())])
        .expect("get_secret succeeds");

    assert!(value.contains_secret());
    assert!(!format!("{value:?}").contains(SECRET_CANARY));

    std::env::remove_var(name);
}

#[test]
fn std_secrets_distinguishes_missing_and_required_values() {
    let _guard = env_lock();
    let name = "NTNT_TEST_STD_SECRET_MISSING";
    std::env::remove_var("NTNT_SECRETS_PROVIDER");
    std::env::remove_var(name);

    let module = secrets::init();
    let optional = native_fn(&module, "get_secret")(&[Value::String(name.to_string())])
        .expect("optional lookup succeeds");
    assert!(matches!(
        optional,
        Value::EnumValue {
            ref enum_name,
            ref variant,
            ..
        } if enum_name == "Option" && variant == "None"
    ));

    let err = native_fn(&module, "require_secret")(&[Value::String(name.to_string())])
        .expect_err("required lookup must fail");
    assert!(err.to_string().contains(name));
    assert!(!err.to_string().contains(SECRET_CANARY));
}

#[test]
fn unsupported_provider_fails_without_falling_back_to_env() {
    let _guard = env_lock();
    let name = "NTNT_TEST_STD_SECRET_NO_FALLBACK";
    std::env::set_var(name, SECRET_CANARY);
    std::env::set_var("NTNT_SECRETS_PROVIDER", "unsupported");

    let module = secrets::init();
    let err = native_fn(&module, "get_secret")(&[Value::String(name.to_string())])
        .expect_err("unsupported provider must fail closed");
    let rendered = err.to_string();
    assert!(rendered.to_lowercase().contains("unsupported"));
    assert!(!rendered.contains(SECRET_CANARY));

    std::env::remove_var("NTNT_SECRETS_PROVIDER");
    std::env::remove_var(name);
}

#[test]
fn defensive_json_conversion_recursively_redacts_secrets() {
    let mut payload = HashMap::new();
    payload.insert(
        "token".to_string(),
        Value::some(Value::Secret(
            SecretValue::new("API_KEY", SECRET_CANARY).expect("valid secret"),
        )),
    );

    let json = intent_value_to_json(&Value::Map(payload));
    let rendered = json.to_string();
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains(SECRET_CANARY));
}

#[test]
fn public_json_conversion_rejects_nested_secrets() {
    let value = Value::Array(vec![Value::Map(HashMap::from([(
        "token".to_string(),
        Value::Secret(SecretValue::new("API_KEY", SECRET_CANARY).expect("valid secret")),
    )]))]);

    let err = intent_value_to_json_reject(&value).expect_err("secret must not serialize");
    let rendered = err.to_string();
    assert!(rendered.contains("Secret"));
    assert!(!rendered.contains(SECRET_CANARY));
}

#[test]
fn stringify_and_server_json_reject_secret_values() {
    let secret = Value::Secret(SecretValue::new("API_KEY", SECRET_CANARY).expect("valid secret"));

    let json_module = crate::stdlib::json::init();
    let stringify_err = native_fn(&json_module, "stringify")(&[secret.clone()])
        .expect_err("stringify must reject secrets");
    assert!(!stringify_err.to_string().contains(SECRET_CANARY));

    let server_module = crate::stdlib::http_server::init();
    let response_err = native_fn(&server_module, "json")(&[Value::Map(HashMap::from([(
        "token".to_string(),
        secret,
    )]))])
    .expect_err("server JSON must reject secrets");
    assert!(!response_err.to_string().contains(SECRET_CANARY));
}

fn fetch_and_capture(mut options: HashMap<String, Value>) -> (Value, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
    let address = listener.local_addr().expect("listener address");
    options.insert(
        "url".to_string(),
        Value::String(format!("http://{address}/capture")),
    );

    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let capture = thread::spawn(move || {
        let started = Instant::now();
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < Duration::from_secs(5) =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept request: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set timeout");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut expected_len = None;

        loop {
            let count = stream.read(&mut buffer).expect("read request");
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);

            if expected_len.is_none() {
                if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..header_end]);
                    let content_len = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    expected_len = Some(header_end + 4 + content_len);
                }
            }

            if expected_len.is_some_and(|length| bytes.len() >= length) {
                break;
            }
        }

        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .expect("write response");
        String::from_utf8(bytes).expect("request is utf-8")
    });

    let result = crate::stdlib::http::http_fetch_with_app_env(&options, Some("development"))
        .expect("fetch executes");
    (result, capture.join().expect("capture thread"))
}

fn test_secret() -> Value {
    Value::Secret(SecretValue::new("API_KEY", SECRET_CANARY).expect("valid secret"))
}

#[test]
fn http_fetch_exposes_secrets_only_in_approved_header_auth_cookie_and_json_fields() {
    let mut options = HashMap::new();
    options.insert("method".to_string(), Value::String("POST".to_string()));
    options.insert(
        "headers".to_string(),
        Value::Map(HashMap::from([("x-api-key".to_string(), test_secret())])),
    );
    options.insert(
        "cookies".to_string(),
        Value::Map(HashMap::from([("session".to_string(), test_secret())])),
    );
    options.insert(
        "auth".to_string(),
        Value::Map(HashMap::from([
            ("user".to_string(), Value::String("client".to_string())),
            ("pass".to_string(), test_secret()),
        ])),
    );
    options.insert(
        "json".to_string(),
        Value::Map(HashMap::from([("token".to_string(), test_secret())])),
    );

    let (_result, request) = fetch_and_capture(options);
    let lower = request.to_lowercase();
    assert!(lower.contains(&format!("x-api-key: {}", SECRET_CANARY)));
    assert!(lower.contains(&format!("cookie: session={}", SECRET_CANARY)));
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("client:{SECRET_CANARY}").as_bytes())
        .to_lowercase();
    assert!(lower.contains(&format!("authorization: basic {basic}")));
    assert!(request.contains(&format!(r#"{{"token":"{}"}}"#, SECRET_CANARY)));
    assert!(!request.contains("[REDACTED]"));
}

#[test]
fn http_fetch_exposes_secret_as_raw_body() {
    let options = HashMap::from([
        ("method".to_string(), Value::String("POST".to_string())),
        ("body".to_string(), test_secret()),
    ]);

    let (_result, request) = fetch_and_capture(options);
    assert!(request.ends_with(SECRET_CANARY));
    assert!(!request.contains("[REDACTED]"));
}

#[test]
fn http_fetch_exposes_secret_as_form_value() {
    let options = HashMap::from([
        ("method".to_string(), Value::String("POST".to_string())),
        (
            "form".to_string(),
            Value::Map(HashMap::from([("token".to_string(), test_secret())])),
        ),
    ]);

    let (_result, request) = fetch_and_capture(options);
    assert!(request.ends_with(&format!("token={SECRET_CANARY}")));
    assert!(!request.contains("[REDACTED]"));
}

#[test]
fn http_fetch_does_not_follow_redirects_when_request_contains_secrets() {
    let target = TcpListener::bind("127.0.0.1:0").expect("bind redirect target");
    let target_addr = target.local_addr().expect("target address");
    target.set_nonblocking(true).expect("nonblocking target");
    let target_request = thread::spawn(move || {
        for _ in 0..100 {
            match target.accept() {
                Ok((mut stream, _)) => {
                    let mut request = Vec::new();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .expect("target read timeout");
                    stream.read_to_end(&mut request).ok();
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .ok();
                    return Some(String::from_utf8_lossy(&request).to_string());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("target accept failed: {error}"),
            }
        }
        None
    });

    let redirect = TcpListener::bind("127.0.0.1:0").expect("bind redirect server");
    let redirect_addr = redirect.local_addr().expect("redirect address");
    let redirect_server = thread::spawn(move || {
        let (mut stream, _) = redirect.accept().expect("accept redirect request");
        let response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target_addr}/target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .expect("write redirect");
    });

    let mut options = HashMap::new();
    options.insert(
        "url".to_string(),
        Value::String(format!("http://{redirect_addr}/start")),
    );
    options.insert(
        "headers".to_string(),
        Value::Map(HashMap::from([("x-api-key".to_string(), test_secret())])),
    );
    crate::stdlib::http::http_fetch_with_app_env(&options, Some("development"))
        .expect("fetch redirect response");

    redirect_server.join().expect("redirect server");
    let forwarded = target_request.join().expect("target thread");
    assert!(
        forwarded.is_none(),
        "secret-bearing request followed redirect: {forwarded:?}"
    );
}

#[test]
fn generic_stringification_sinks_reject_secrets_without_leaking_them() {
    let cases = [
        (
            "csv.stringify",
            native_fn(&crate::stdlib::csv::init(), "stringify")(&[Value::Array(vec![
                Value::Array(vec![test_secret()]),
            ])]),
        ),
        (
            "url.build_query",
            native_fn(&crate::stdlib::url::init(), "build_query")(&[Value::Map(HashMap::from([
                ("token".to_string(), test_secret()),
            ]))]),
        ),
        (
            "string.join",
            native_fn(&crate::stdlib::string::init(), "join")(&[
                Value::Array(vec![test_secret()]),
                Value::String(",".to_string()),
            ]),
        ),
        (
            "string.concat",
            native_fn(&crate::stdlib::string::init(), "concat")(&[
                Value::Array(vec![test_secret()]),
                Value::String(String::new()),
            ]),
        ),
        (
            "collections.sort",
            native_fn(&crate::stdlib::collections::init(), "sort")(&[Value::Array(vec![
                test_secret(),
            ])]),
        ),
    ];

    for (name, result) in cases {
        let error = result.expect_err(&format!("{name} must reject secrets"));
        let rendered = error.to_string();
        assert!(rendered.contains("Secret"), "{name}: {rendered}");
        assert!(!rendered.contains(SECRET_CANARY), "{name}: {rendered}");
    }
}
