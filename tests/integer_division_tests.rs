#[path = "support/system_fixture.rs"]
mod fixture;

fn assert_runtime_error(expression: &str, message: &str) {
    let source = format!("let minimum = -9223372036854775807 - 1\nprint({expression})");
    let mut run = fixture::Fixture::start(&source, &["run"], &[]);
    let status = run.wait();
    let stderr = run.stderr();
    assert_eq!(status.code(), Some(1), "{expression}: {stderr}");
    assert!(stderr.contains(message), "{expression}: {stderr}");
    assert!(!stderr.contains("panicked"), "{expression}: {stderr}");
}

#[test]
fn integer_division_overflow_is_runtime_error() {
    assert_runtime_error("minimum / -1", "Integer overflow in division");
}

#[test]
fn integer_remainder_overflow_is_runtime_error() {
    assert_runtime_error("minimum % -1", "Integer overflow in remainder");
}

#[test]
fn integer_zero_divisors_are_runtime_errors() {
    for expression in ["minimum / 0", "minimum % 0"] {
        assert_runtime_error(expression, "Division by zero");
    }
}

#[test]
fn integer_division_preserves_valid_boundary_and_signed_results() {
    let output = fixture::strict_run(
        r#"
let minimum = -9223372036854775807 - 1
print(minimum / 1)
print(minimum % 1)
print(minimum % -2)
print(-7 / 3)
print(-7 % 3)
print(7 / -3)
print(7 % -3)
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        ["-9223372036854775808", "0", "0", "-2", "-1", "-2", "1"]
    );
}

#[test]
fn integer_division_errors_are_catchable_with_otherwise() {
    let output = fixture::strict_run(
        r#"
fn recover(divisor: Int, remainder: Bool) -> String {
    let minimum = -9223372036854775807 - 1
    if remainder {
        let value = minimum % divisor otherwise { return "caught remainder" }
        return str(value)
    }
    let value = minimum / divisor otherwise { return "caught division" }
    return str(value)
}
print(recover(-1, false))
print(recover(-1, true))
print(recover(0, false))
print(recover(0, true))
print(recover(1, false))
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        [
            "caught division",
            "caught remainder",
            "caught division",
            "caught remainder",
            "-9223372036854775808",
        ]
    );
}

fn assert_server_survives_integer_division_errors(workers: &str) {
    let source = r#"
import { text } from "std/http/server"
let minimum = -9223372036854775807 - 1
get("/health", fn(req) { return text("healthy") })
get("/division-overflow", fn(req) { return text(str(minimum / -1)) })
get("/remainder-overflow", fn(req) { return text(str(minimum % -1)) })
get("/division-zero", fn(req) { return text(str(minimum / 0)) })
get("/remainder-zero", fn(req) { return text(str(minimum % 0)) })
listen(0, map { "fixture": true, "readiness": "json" })
"#;
    let mut server = fixture::Fixture::start(
        source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", workers)],
    );
    let addr = server.ready();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    for path in [
        "/division-overflow",
        "/remainder-overflow",
        "/division-zero",
        "/remainder-zero",
    ] {
        let response = client
            .get(format!("http://{addr}{path}"))
            .send()
            .unwrap_or_else(|error| panic!("{path}: {error}; {}", server.stderr()));
        assert_eq!(response.status(), 500, "{path}: {}", server.stderr());
        let health = client.get(format!("http://{addr}/health")).send().unwrap();
        assert_eq!(health.status(), 200);
        assert_eq!(health.text().unwrap(), "healthy");
        assert!(server.child.try_wait().unwrap().is_none());
    }
    let stderr = server.stderr();
    for message in [
        "Integer overflow in division",
        "Integer overflow in remainder",
        "Division by zero",
    ] {
        assert!(stderr.contains(message), "{stderr}");
    }
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn integer_division_errors_preserve_single_worker_server() {
    assert_server_survives_integer_division_errors("1");
}

#[test]
fn integer_division_errors_preserve_multi_worker_server() {
    assert_server_survives_integer_division_errors("3");
}
