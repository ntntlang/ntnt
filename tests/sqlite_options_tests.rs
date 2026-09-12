//! Public SQLite options and real cross-process migration serialization.
use ntnt::interpreter::{Interpreter, Value};
use ntnt::stdlib::sqlite;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn opts(key: &str, value: Value) -> Value {
    Value::Map(HashMap::from([(key.into(), value)]))
}
fn call(name: &str, args: &[Value]) -> Value {
    match sqlite::init().remove(name).unwrap() {
        Value::NativeFunction { func, .. } => func(args).unwrap(),
        _ => unreachable!(),
    }
}
fn ok(value: Value) -> Value {
    match value {
        Value::EnumValue {
            enum_name,
            variant,
            mut values,
        } if enum_name == "Result" && variant == "Ok" => values.remove(0),
        other => panic!("expected Ok, got {other:?}"),
    }
}
struct Db(Value);
impl Drop for Db {
    fn drop(&mut self) {
        let _ = call("close", std::slice::from_ref(&self.0));
    }
}
fn execute(db: &Db, sql: &str) {
    ok(call(
        "execute",
        &[
            db.0.clone(),
            Value::String(sql.into()),
            Value::Array(vec![]),
        ],
    ));
}
fn applied(db: &Db) -> i64 {
    let row = ok(call(
        "query_one",
        &[
            db.0.clone(),
            Value::String("SELECT count(*) AS n FROM migrations".into()),
            Value::Array(vec![]),
        ],
    ));
    let Value::Map(row) = row else {
        panic!("expected row")
    };
    let Value::Int(n) = row["n"] else {
        panic!("expected count")
    };
    n
}

#[test]
fn sqlite_options_interpreter_and_typechecker_accept_optional_maps() {
    let source = r#"
import { connect, begin, commit, rollback, query_one, close } from "std/db/sqlite"
let old = unwrap(connect(":memory:"))
unwrap(begin(old))
rollback(old)
close(old)
let db = unwrap(connect(":memory:", map { "busy_timeout_ms": 125 }))
unwrap(begin(db, map { "mode": "immediate" }))
rollback(db)
unwrap(begin(db, map { "mode": "exclusive" }))
commit(db)
let row = unwrap(query_one(db, "PRAGMA busy_timeout", []))
close(db)
row
"#;
    let ast = ntnt::parser::Parser::new(ntnt::lexer::Lexer::new(source).collect())
        .parse()
        .unwrap();
    let errors: Vec<_> = ntnt::typechecker::check_program(&ast, source)
        .into_iter()
        .filter(|d| d.severity == ntnt::typechecker::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let value = Interpreter::new().eval(&ast).unwrap();
    let Value::Map(row) = value else {
        panic!("expected row, got {value:?}")
    };
    assert!(
        matches!(row.get("timeout"), Some(Value::Int(125))),
        "{row:?}"
    );
}

#[test]
fn sqlite_options_typechecker_rejects_wrong_types_and_arity() {
    for call in [
        "connect()",
        "connect(12)",
        "connect(\":memory:\", 12)",
        "connect(\":memory:\", map {}, map {})",
        "begin()",
        "begin(db, \"immediate\")",
        "begin(db, map {}, map {})",
    ] {
        let source = format!("import {{ connect, begin }} from \"std/db/sqlite\"\nlet db = unwrap(connect(\":memory:\"))\n{call}");
        let ast = ntnt::parser::Parser::new(ntnt::lexer::Lexer::new(&source).collect())
            .parse()
            .unwrap();
        let errors: Vec<_> = ntnt::typechecker::check_program(&ast, &source)
            .into_iter()
            .filter(|d| d.severity == ntnt::typechecker::Severity::Error)
            .collect();
        assert!(!errors.is_empty(), "accepted {call}");
    }
}

struct Control(BufReader<TcpStream>);
impl Control {
    fn new(stream: TcpStream) -> Self {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Self(BufReader::new(stream))
    }
    fn send(&mut self, message: &str) {
        writeln!(self.0.get_mut(), "{message}").unwrap();
        self.0.get_mut().flush().unwrap();
    }
    fn receive(&mut self) -> std::io::Result<String> {
        let mut line = String::new();
        if self.0.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "worker closed control socket",
            ));
        }
        Ok(line.trim_end().into())
    }
    fn expect(&mut self, expected: &str) {
        assert_eq!(self.receive().unwrap(), expected);
    }
}
struct Process {
    child: Child,
    output: std::fs::File,
}
impl Process {
    fn start(role: &str, path: &std::path::Path, address: std::net::SocketAddr) -> Self {
        let output = tempfile::tempfile().unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sqlite_migration_process_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("NTNT_SQLITE_MIGRATION_ROLE", role)
            .env("NTNT_SQLITE_MIGRATION_PATH", path)
            .env("NTNT_SQLITE_MIGRATION_CONTROL", address.to_string())
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output.try_clone().unwrap()))
            .spawn()
            .unwrap();
        Self { child, output }
    }
    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                self.output.rewind().unwrap();
                let mut output = String::new();
                self.output.read_to_string(&mut output).unwrap();
                assert!(status.success(), "worker failed: {output}");
                return;
            }
            assert!(Instant::now() < deadline, "worker failed to terminate");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn accept(listener: &TcpListener, worker: &mut Process) -> Control {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Control::new(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if worker.child.try_wait().unwrap().is_some() {
                    worker.finish();
                    panic!("worker exited before connecting");
                }
                assert!(Instant::now() < deadline, "worker never connected");
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    }
}

#[test]
fn sqlite_immediate_serializes_two_process_migration_check_and_ddl() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("migrations.db");
    {
        let setup = rusqlite::Connection::open(&path).unwrap();
        setup
            .execute_batch(
                "PRAGMA journal_mode=WAL; CREATE TABLE migrations (version INTEGER PRIMARY KEY)",
            )
            .unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut holder = Process::start("holder", &path, listener.local_addr().unwrap());
    let mut first = accept(&listener, &mut holder);
    first.expect("ready holder");
    let mut contender = Process::start("contender", &path, listener.local_addr().unwrap());
    let mut second = accept(&listener, &mut contender);
    second.expect("ready contender");

    first.send("begin");
    first.expect("acquired 0");
    second.send("begin");
    second.expect("attempting");
    second
        .0
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let error = second
        .receive()
        .expect_err("contender must not read the migration ledger before holder commits");
    assert!(
        matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
        "{error}"
    );
    second
        .0
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    first.send("commit");
    first.expect("applied");
    second.expect("acquired 1");
    second.expect("skipped");
    holder.finish();
    contender.finish();

    let verify = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        verify
            .query_row("SELECT count(*) FROM migrations WHERE version=1", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        verify
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='projects'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
#[ignore = "helper runs only in the two-process parent regression"]
fn sqlite_migration_process_worker() {
    let role = std::env::var("NTNT_SQLITE_MIGRATION_ROLE").expect("worker role");
    let path = std::env::var("NTNT_SQLITE_MIGRATION_PATH").expect("worker database");
    let address = std::env::var("NTNT_SQLITE_MIGRATION_CONTROL").expect("control address");
    let db = Db(ok(call(
        "connect",
        &[
            Value::String(path),
            opts("busy_timeout_ms", Value::Int(5000)),
        ],
    )));
    let mut control = Control::new(TcpStream::connect(address).unwrap());
    control.send(&format!("ready {role}"));
    control.expect("begin");
    if role == "contender" {
        control.send("attempting");
    }
    ok(call(
        "begin",
        &[
            db.0.clone(),
            opts("mode", Value::String("immediate".into())),
        ],
    ));
    let count = applied(&db);
    control.send(&format!("acquired {count}"));
    if role == "holder" {
        control.expect("commit");
    }
    if count == 0 {
        // No IF NOT EXISTS: a second application of this DDL must fail the test.
        execute(&db, "CREATE TABLE projects (id INTEGER PRIMARY KEY)");
        execute(&db, "INSERT INTO migrations (version) VALUES (1)");
    }
    assert!(matches!(
        call("commit", std::slice::from_ref(&db.0)),
        Value::Bool(true)
    ));
    control.send(if count == 0 { "applied" } else { "skipped" });
}
