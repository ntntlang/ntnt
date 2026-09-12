use super::*;

fn call(name: &str, args: Vec<Value>) -> Result<Value> {
    let module = init();
    match &module[name] {
        Value::NativeFunction { func, .. } => func(&args),
        _ => panic!("expected native function"),
    }
}

fn options(key: &str, value: Value) -> Value {
    Value::Map(HashMap::from([(key.to_string(), value)]))
}

fn unwrap_ok(result: Result<Value>) -> Value {
    match result.unwrap() {
        Value::EnumValue {
            enum_name,
            variant,
            mut values,
            ..
        } if enum_name == "Result" && variant == "Ok" => values.remove(0),
        other => panic!("expected Result::Ok, got {other:?}"),
    }
}

struct TestConnection(Value);

impl TestConnection {
    fn open(path: &str, opts: Option<Value>) -> Self {
        let mut args = vec![Value::String(path.to_string())];
        args.extend(opts);
        Self(unwrap_ok(call("connect", args)))
    }

    fn begin(&self, opts: Option<Value>) -> Result<Value> {
        let mut args = vec![self.0.clone()];
        args.extend(opts);
        call("begin", args)
    }

    fn is_autocommit(&self) -> bool {
        get_connection(&self.0)
            .unwrap()
            .lock()
            .unwrap()
            .is_autocommit()
    }

    fn timeout(&self) -> i64 {
        get_connection(&self.0)
            .unwrap()
            .lock()
            .unwrap()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap()
    }
}

impl Drop for TestConnection {
    fn drop(&mut self) {
        sqlite_close(&self.0).unwrap();
    }
}

fn unwrap_err(result: Result<Value>) -> String {
    match result.unwrap() {
        Value::EnumValue {
            enum_name,
            variant,
            values,
            ..
        } if enum_name == "Result" && variant == "Err" => match &values[0] {
            Value::String(message) => message.clone(),
            other => panic!("expected String error, got {other:?}"),
        },
        other => panic!("expected Result::Err, got {other:?}"),
    }
}

fn secret() -> Value {
    Value::Secret(
        crate::interpreter::SecretValue::new("OPTION_SECRET", "option-secret-canary").unwrap(),
    )
}

#[test]
fn connect_invalid_options_rejected_before_file_creation_without_input_leaks() {
    let invalid = vec![
        Value::Unit,
        Value::Int(0),
        Value::String("option-secret-canary".into()),
        Value::Array(vec![]),
        secret(),
        options("secret-key-canary", Value::Int(0)),
        options("busy_timeout_ms", Value::Int(-1)),
        options("busy_timeout_ms", Value::Int(i32::MAX as i64 + 1)),
        options("busy_timeout_ms", Value::Int(i64::MAX)),
        options("busy_timeout_ms", Value::Float(1.0)),
        options(
            "busy_timeout_ms",
            Value::String("option-secret-canary".into()),
        ),
        options("busy_timeout_ms", Value::Bool(false)),
        options("busy_timeout_ms", Value::Unit),
        options("busy_timeout_ms", secret()),
        options("busy_timeout_ms", options("secret-key-canary", secret())),
        Value::Map(HashMap::from([
            ("busy_timeout_ms".into(), Value::Int(0)),
            ("secret-key-canary".into(), secret()),
        ])),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (index, opts) in invalid.into_iter().enumerate() {
        let path = dir.path().join(format!("not-created-{index}.db"));
        let message = unwrap_err(call(
            "connect",
            vec![Value::String(path.to_str().unwrap().into()), opts],
        ));
        assert!(message.contains("connect"), "{message}");
        assert!(
            message.contains("map") || message.contains("busy_timeout_ms"),
            "{message}"
        );
        for canary in [
            "option-secret-canary",
            "secret-key-canary",
            "OPTION_SECRET",
            "not-created-",
        ] {
            assert!(!message.contains(canary), "{message}");
        }
        assert!(!path.exists(), "invalid options created a database");
    }
}

#[test]
fn connect_explicit_timeout_reports_wal_setup_failure_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup.db");
    let peer = Connection::open(&path).unwrap();
    peer.execute_batch("CREATE TABLE existing (id INTEGER); BEGIN; SELECT * FROM existing;")
        .unwrap();
    let error = unwrap_err(call(
        "connect",
        vec![
            Value::String(path.to_str().unwrap().into()),
            options("busy_timeout_ms", Value::Int(0)),
        ],
    ));
    assert!(error.contains("WAL setup failed"), "{error}");
    peer.execute_batch("ROLLBACK").unwrap();
    let db = TestConnection::open(
        path.to_str().unwrap(),
        Some(options("busy_timeout_ms", Value::Int(0))),
    );
    assert_eq!(db.timeout(), 0);
    let connection = get_connection(&db.0).unwrap();
    let connection = connection.lock().unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        connection
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

// Rollback journal mode distinguishes EXCLUSIVE from IMMEDIATE (WAL intentionally does not).
fn locking_pair() -> (tempfile::TempDir, TestConnection, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("locking.db");
    let conn = TestConnection::open(
        path.to_str().unwrap(),
        Some(options("busy_timeout_ms", Value::Int(0))),
    );
    get_connection(&conn.0)
        .unwrap()
        .lock()
        .unwrap()
        .execute_batch("PRAGMA journal_mode=DELETE; CREATE TABLE items (id INTEGER);")
        .unwrap();
    let peer = Connection::open(path).unwrap();
    peer.busy_timeout(std::time::Duration::ZERO).unwrap();
    (dir, conn, peer)
}

#[test]
fn begin_default_empty_and_deferred_do_not_reserve_writer() {
    for opts in [
        None,
        Some(Value::Map(HashMap::new())),
        Some(options("mode", Value::String("deferred".into()))),
    ] {
        let (_dir, conn, peer) = locking_pair();
        let returned = unwrap_ok(conn.begin(opts));
        assert!(Arc::ptr_eq(
            &get_connection(&returned).unwrap(),
            &get_connection(&conn.0).unwrap()
        ));
        assert!(!conn.is_autocommit());
        peer.execute_batch("BEGIN IMMEDIATE; INSERT INTO items VALUES (1); ROLLBACK")
            .unwrap();
        assert!(matches!(
            sqlite_rollback(&conn.0).unwrap(),
            Value::Bool(true)
        ));
        assert!(conn.is_autocommit());
    }
}

#[test]
fn begin_immediate_reserves_writer_but_allows_reader() {
    let (_dir, conn, peer) = locking_pair();
    let returned = unwrap_ok(conn.begin(Some(options("mode", Value::String("immediate".into())))));
    assert!(Arc::ptr_eq(
        &get_connection(&returned).unwrap(),
        &get_connection(&conn.0).unwrap()
    ));
    assert!(!conn.is_autocommit());
    assert!(
        peer.execute_batch("BEGIN IMMEDIATE").is_err(),
        "immediate must reserve the writer before any write"
    );
    assert!(peer.is_autocommit());
    assert_eq!(
        peer.query_row("SELECT count(*) FROM items", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(matches!(
        sqlite_rollback(&conn.0).unwrap(),
        Value::Bool(true)
    ));
    peer.execute_batch("BEGIN IMMEDIATE; ROLLBACK").unwrap();
}

#[test]
fn begin_exclusive_blocks_reader_in_rollback_journal_mode() {
    let (_dir, conn, peer) = locking_pair();
    let returned = unwrap_ok(conn.begin(Some(options("mode", Value::String("exclusive".into())))));
    assert!(Arc::ptr_eq(
        &get_connection(&returned).unwrap(),
        &get_connection(&conn.0).unwrap()
    ));
    assert!(!conn.is_autocommit());
    assert!(
        peer.query_row("SELECT count(*) FROM items", [], |row| row.get::<_, i64>(0))
            .is_err(),
        "exclusive must block readers outside WAL mode"
    );
    assert!(matches!(
        sqlite_rollback(&conn.0).unwrap(),
        Value::Bool(true)
    ));
    assert_eq!(
        peer.query_row("SELECT count(*) FROM items", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn begin_lock_failure_leaves_no_transaction() {
    for mode in ["immediate", "exclusive"] {
        let (_dir, conn, peer) = locking_pair();
        peer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let message = unwrap_err(conn.begin(Some(options("mode", Value::String(mode.into())))));
        assert!(message.contains("BEGIN failed"), "{message}");
        assert!(conn.is_autocommit());
        peer.execute_batch("ROLLBACK").unwrap();
        unwrap_ok(conn.begin(Some(options("mode", Value::String(mode.into())))));
        assert!(matches!(
            sqlite_rollback(&conn.0).unwrap(),
            Value::Bool(true)
        ));
    }
}

#[test]
fn begin_invalid_options_leave_no_transaction_or_input_leaks() {
    let conn = TestConnection::open(":memory:", None);
    let invalid = vec![
        Value::Unit,
        Value::Int(0),
        Value::String("option-secret-canary".into()),
        Value::Array(vec![]),
        secret(),
        options("secret-key-canary", Value::String("immediate".into())),
        options("mode", Value::String("".into())),
        options("mode", Value::String("IMMEDIATE".into())),
        options("mode", Value::String(" immediate ".into())),
        options(
            "mode",
            Value::String("immediate; CREATE TABLE option_secret_canary(x)".into()),
        ),
        options("mode", Value::Int(1)),
        options("mode", Value::Float(1.0)),
        options("mode", Value::Bool(true)),
        options("mode", Value::Unit),
        options("mode", secret()),
        options("mode", options("secret-key-canary", secret())),
        Value::Map(HashMap::from([
            ("mode".into(), Value::String("immediate".into())),
            ("secret-key-canary".into(), secret()),
        ])),
    ];
    for opts in invalid {
        let message = unwrap_err(conn.begin(Some(opts)));
        assert!(message.contains("begin"), "{message}");
        assert!(
            message.contains("map") || message.contains("mode"),
            "{message}"
        );
        for canary in [
            "option-secret-canary",
            "option_secret_canary",
            "secret-key-canary",
            "OPTION_SECRET",
            "IMMEDIATE",
        ] {
            assert!(!message.contains(canary), "{message}");
        }
        assert!(conn.is_autocommit());
    }
}

#[test]
fn begin_nested_and_closed_handle_keep_existing_errors() {
    for opts in [
        None,
        Some(options("mode", Value::String("immediate".into()))),
    ] {
        let conn = TestConnection::open(":memory:", None);
        unwrap_ok(conn.begin(opts.clone()));
        let message = unwrap_err(conn.begin(opts.clone()));
        assert!(message.contains("BEGIN failed"));
        assert!(
            !conn.is_autocommit(),
            "nested failure must not end the existing transaction"
        );
        assert!(matches!(
            sqlite_rollback(&conn.0).unwrap(),
            Value::Bool(true)
        ));
        sqlite_close(&conn.0).unwrap();
        let error = conn.begin(opts).unwrap_err();
        assert!(error
            .to_string()
            .contains("Invalid or closed SQLite connection"));
    }
}

#[test]
fn native_option_arity_is_one_required_two_maximum() {
    let module = init();
    for name in ["connect", "begin"] {
        match &module[name] {
            Value::NativeFunction {
                arity, max_arity, ..
            } => assert_eq!((*arity, *max_arity), (1, 2)),
            _ => panic!("expected native function"),
        }
    }
    assert!(call("connect", vec![Value::Int(123)]).is_err());
}

#[test]
fn connect_timeout_defaults_and_explicit_bounds() {
    assert_eq!(TestConnection::open(":memory:", None).timeout(), 5000);
    assert_eq!(
        TestConnection::open(":memory:", Some(Value::Map(HashMap::new()))).timeout(),
        5000
    );
    for milliseconds in [0, 7, 5000, i32::MAX as i64] {
        let conn = TestConnection::open(
            ":memory:",
            Some(options("busy_timeout_ms", Value::Int(milliseconds))),
        );
        assert_eq!(conn.timeout(), milliseconds);
    }
}
