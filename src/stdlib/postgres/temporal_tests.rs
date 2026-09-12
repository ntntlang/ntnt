use super::*;
use tokio_postgres::types::{private::BytesMut, FromSql, IsNull, Type};

fn encoded(ty: &Type, input: &str) -> BytesMut {
    let mut bytes = BytesMut::new();
    let result = SqlParam::String(input.into())
        .to_sql_checked(ty, &mut bytes)
        .unwrap_or_else(|e| panic!("{ty} {input:?}: {e}"));
    assert!(matches!(result, IsNull::No));
    bytes
}

#[test]
fn temporal_timestamptz_binary_encoding() {
    for input in [
        "2026-07-29T12:54:21.000000Z",
        "2026-07-29T06:54:21.123456-06:00",
        "2026-07-29T18:39:21.123456+05:45",
        "1999-12-31T23:59:59.999999Z",
        "2000-01-01T00:00:00Z",
    ] {
        let bytes = encoded(&Type::TIMESTAMPTZ, input);
        assert_eq!(
            bytes.len(),
            8,
            "TIMESTAMPTZ is binary microseconds, not UTF-8"
        );
        let actual = DateTime::<Utc>::from_sql(&Type::TIMESTAMPTZ, &bytes).unwrap();
        assert_eq!(actual, DateTime::parse_from_rfc3339(input).unwrap());
    }
}

#[test]
fn temporal_date_time_timestamp_binary_encoding() {
    for input in ["2024-02-29", "1999-12-31", "2000-01-01"] {
        let bytes = encoded(&Type::DATE, input);
        assert_eq!(bytes.len(), 4);
        assert_eq!(
            NaiveDate::from_sql(&Type::DATE, &bytes).unwrap(),
            NaiveDate::parse_from_str(input, "%Y-%m-%d").unwrap()
        );
    }
    for input in ["00:00:00", "12:54:21.123456", "23:59:59.999999"] {
        let bytes = encoded(&Type::TIME, input);
        assert_eq!(bytes.len(), 8);
        assert_eq!(
            NaiveTime::from_sql(&Type::TIME, &bytes).unwrap(),
            NaiveTime::parse_from_str(input, "%H:%M:%S%.f").unwrap()
        );
    }
    for input in [
        "2024-02-29T12:54:21.123456",
        "1999-12-31 23:59:59.999999",
        "2000-01-01T00:00:00",
    ] {
        let bytes = encoded(&Type::TIMESTAMP, input);
        assert_eq!(bytes.len(), 8);
        assert_eq!(
            NaiveDateTime::from_sql(&Type::TIMESTAMP, &bytes).unwrap(),
            NaiveDateTime::parse_from_str(&input.replace('T', " "), "%Y-%m-%d %H:%M:%S%.f")
                .unwrap()
        );
    }
}

fn invalid_inputs() -> Vec<(Type, &'static str)> {
    vec![
        (Type::DATE, ""),
        (Type::DATE, "2023-02-29"),
        (Type::DATE, "2024-13-01"),
        (Type::DATE, "2024-02-29T00:00:00Z"),
        (Type::DATE, "infinity"),
        (Type::TIME, ""),
        (Type::TIME, "24:00:00"),
        (Type::TIME, "12:00:00Z"),
        (Type::TIME, "12:00:00-06:00"),
        (Type::TIME, "23:59:60"),
        (Type::TIMESTAMP, ""),
        (Type::TIMESTAMP, "2023-02-29T00:00:00"),
        (Type::TIMESTAMP, "2026-07-29T12:00:00Z"),
        (Type::TIMESTAMP, "2016-12-31T23:59:60"),
        (Type::TIMESTAMPTZ, ""),
        (Type::TIMESTAMPTZ, "2026-07-29T12:00:00"),
        (Type::TIMESTAMPTZ, "2026-07-29T12:00:00+25:00"),
        (Type::TIMESTAMPTZ, "2026-07-29T12:00:00 America/Denver"),
        (Type::TIMESTAMPTZ, "2016-12-31T23:59:60Z"),
        (Type::TIMESTAMPTZ, "infinity"),
        (Type::TIMESTAMPTZ, "now"),
        (Type::TIMESTAMPTZ, "private-bound-value-canary"),
    ]
}

#[test]
fn temporal_invalid_strings_rejected_before_writing_bytes() {
    for (ty, input) in invalid_inputs() {
        let mut bytes = BytesMut::from(&b"prefix"[..]);
        let error = SqlParam::String(input.into())
            .to_sql_checked(&ty, &mut bytes)
            .err()
            .unwrap_or_else(|| panic!("{ty} accepted {input:?}"));
        let message = error.to_string();
        assert!(
            message.contains(&format!(
                "Cannot coerce string to {}",
                ty.name().to_uppercase()
            )),
            "{message}"
        );
        assert!(!message.contains("private-bound-value-canary"));
        assert_eq!(
            &bytes[..],
            b"prefix",
            "failed parsing must not emit parameter bytes"
        );
    }
}

#[test]
fn temporal_submicrosecond_precision_is_truncated_even_before_epoch() {
    for input in [
        "2026-07-29T12:54:21.123456789",
        "1999-12-31T23:59:59.999999999",
    ] {
        let actual =
            NaiveDateTime::from_sql(&Type::TIMESTAMP, &encoded(&Type::TIMESTAMP, input)).unwrap();
        let expected = NaiveDateTime::parse_from_str(&input[..26], "%Y-%m-%dT%H:%M:%S%.f").unwrap();
        assert_eq!(actual, expected);
        let zoned = format!("{input}Z");
        let actual =
            DateTime::<Utc>::from_sql(&Type::TIMESTAMPTZ, &encoded(&Type::TIMESTAMPTZ, &zoned))
                .unwrap();
        assert_eq!(actual.naive_utc(), expected);
    }
    assert_eq!(
        NaiveTime::from_sql(&Type::TIME, &encoded(&Type::TIME, "12:54:21.123456789")).unwrap(),
        NaiveTime::parse_from_str("12:54:21.123456", "%H:%M:%S%.f").unwrap()
    );
}

#[test]
fn temporal_null_and_existing_string_encodings_unchanged() {
    for ty in [Type::DATE, Type::TIME, Type::TIMESTAMP, Type::TIMESTAMPTZ] {
        for value in [Value::Unit, Value::none()] {
            let mut bytes = BytesMut::new();
            assert!(matches!(
                value_to_sql_param(&value)
                    .unwrap()
                    .to_sql_checked(&ty, &mut bytes)
                    .unwrap(),
                IsNull::Yes
            ));
            assert!(bytes.is_empty());
        }
    }
    assert_eq!(&encoded(&Type::TEXT, "not a date")[..], b"not a date");
    assert_eq!(
        i32::from_sql(&Type::INT4, &encoded(&Type::INT4, "42")).unwrap(),
        42
    );
    assert!(bool::from_sql(&Type::BOOL, &encoded(&Type::BOOL, "true")).unwrap());
    let id = "9ad1f4b6-09d7-421c-af08-bf395d28da37";
    assert_eq!(
        uuid::Uuid::from_sql(&Type::UUID, &encoded(&Type::UUID, id))
            .unwrap()
            .to_string(),
        id
    );
    for ty in [Type::JSON, Type::JSONB] {
        assert_eq!(
            serde_json::Value::from_sql(&ty, &encoded(&ty, "{\"n\":1}")).unwrap(),
            serde_json::json!({"n":1})
        );
    }
}

fn test_url() -> String {
    std::env::var("NTNT_POSTGRES_TEST_URL")
        .expect("set NTNT_POSTGRES_TEST_URL to a disposable PostgreSQL database")
}

fn result_ok(value: Value) -> Value {
    match value {
        Value::EnumValue {
            enum_name,
            variant,
            mut values,
        } if enum_name == "Result" && variant == "Ok" => values.remove(0),
        other => panic!("expected Ok, got {other:?}"),
    }
}

struct TestConnection(Value);
impl TestConnection {
    fn open(url: &str) -> Self {
        Self(result_ok(pg_connect(url).expect("connect must succeed")))
    }
}
impl Drop for TestConnection {
    fn drop(&mut self) {
        if has_active_txn(&self.0) {
            let _ = pg_rollback(&self.0);
        }
        let _ = pg_close(&self.0);
    }
}

fn assert_true_map(value: Value) {
    match value {
        Value::Map(row) => assert!(matches!(row.get("ok"), Some(Value::Bool(true))), "{row:?}"),
        other => panic!("expected row map, got {other:?}"),
    }
}
fn assert_true_row(value: Value) {
    match result_ok(value) {
        Value::EnumValue {
            enum_name,
            variant,
            mut values,
        } if enum_name == "Option" && variant == "Some" => assert_true_map(values.remove(0)),
        other => panic!("expected Some row, got {other:?}"),
    }
}

#[test]
#[ignore = "requires NTNT_POSTGRES_TEST_URL"]
fn temporal_postgres_timestamptz_issue_175() {
    let db = TestConnection::open(&test_url());
    assert_true_row(
        pg_query_one(
            &db.0,
            "SELECT $1::timestamptz = TIMESTAMPTZ '2026-07-29 12:54:21+00' AS ok",
            &[Value::String("2026-07-29T12:54:21.000000Z".into())],
        )
        .unwrap(),
    );
}

#[test]
#[ignore = "requires NTNT_POSTGRES_TEST_URL"]
fn temporal_postgres_all_query_paths() {
    let db = TestConnection::open(&test_url());
    for transaction in [false, true] {
        if transaction {
            result_ok(pg_begin(&db.0).unwrap());
        }
        // Repeating each query exercises the prepared statement cache outside transactions.
        for _ in 0..2 {
            for (ty, input, expected) in [
                ("date", "2024-02-29", "2024-02-29"),
                ("time", "12:54:21.123456", "12:54:21.123456"),
                (
                    "timestamp",
                    "1999-12-31T23:59:59.999999",
                    "1999-12-31 23:59:59.999999",
                ),
                (
                    "timestamptz",
                    "2026-07-29T06:54:21.123456-06:00",
                    "2026-07-29 12:54:21.123456+00",
                ),
                (
                    "timestamptz",
                    "2026-07-29T18:39:21.123456+05:45",
                    "2026-07-29 12:54:21.123456+00",
                ),
            ] {
                let sql = format!("SELECT $1::{ty} = '{expected}'::{ty} AS ok");
                let params = [Value::String(input.into())];
                assert_true_row(pg_query_one(&db.0, &sql, &params).unwrap());
                match result_ok(pg_query(&db.0, &sql, &params).unwrap()) {
                    Value::Array(mut rows) => {
                        assert_eq!(rows.len(), 1);
                        assert_true_map(rows.remove(0));
                    }
                    other => panic!("expected rows, got {other:?}"),
                }
                assert!(matches!(
                    result_ok(pg_execute(&db.0, &sql, &params).unwrap()),
                    Value::Int(1)
                ));
                assert_true_row(
                    pg_query_one(
                        &db.0,
                        &format!("SELECT $1::{ty} IS NULL AS ok"),
                        &[Value::none()],
                    )
                    .unwrap(),
                );
            }
        }
        if transaction {
            result_ok(pg_rollback(&db.0).unwrap());
        }
    }
}

#[test]
#[ignore = "requires NTNT_POSTGRES_TEST_URL"]
fn temporal_postgres_invalid_strings_report_safe_errors_and_connection_recovers() {
    let db = TestConnection::open(&test_url());
    for transaction in [false, true] {
        if transaction {
            result_ok(pg_begin(&db.0).unwrap());
        }
        for (ty, input) in invalid_inputs() {
            let sql = format!("SELECT $1::{} AS value", ty.name());
            let params = [Value::String(input.into())];
            for result in [
                pg_query_one(&db.0, &sql, &params),
                pg_query(&db.0, &sql, &params),
                pg_execute(&db.0, &sql, &params),
            ] {
                match result.unwrap() {
                    Value::EnumValue {
                        enum_name,
                        variant,
                        values,
                    } if enum_name == "Result" && variant == "Err" => {
                        let Value::String(message) = &values[0] else {
                            panic!("expected error string")
                        };
                        assert!(
                            message.contains(&format!(
                                "Cannot coerce string to {}",
                                ty.name().to_uppercase()
                            )),
                            "{message}"
                        );
                        assert!(!message.contains("private-bound-value-canary"));
                        assert!(!message.contains("incorrect binary"));
                    }
                    other => panic!("expected Err, got {other:?}"),
                }
            }
            // Client-side rejection must leave even a transaction usable (no server-side bind failure).
            assert_true_row(pg_query_one(&db.0, "SELECT true AS ok", &[]).unwrap());
        }
        if transaction {
            result_ok(pg_rollback(&db.0).unwrap());
        }
    }
}
