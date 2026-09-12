//! Real PostgreSQL pool lifecycle contracts. Each scenario runs in its own process
//! so first-use environment configuration and process-local registries are isolated.
use ntnt::interpreter::Value;
use ntnt::stdlib::postgres as pg;
use std::collections::HashSet;
use std::io::{Read, Seek};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Barrier};
use std::time::{Duration, Instant};

fn call(name: &str, args: &[Value]) -> Value {
    match pg::init().remove(name).unwrap() {
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
fn error(value: Value) -> String {
    match value {
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Result" && variant == "Err" => {
            let Value::String(message) = &values[0] else {
                panic!("expected error string")
            };
            message.clone()
        }
        other => panic!("expected Err, got {other:?}"),
    }
}
struct Db(Value);
impl Db {
    fn open(url: &str) -> Self {
        Self(ok(call("connect", &[Value::String(url.into())])))
    }
    fn row(&self, sql: &str) -> std::collections::HashMap<String, Value> {
        row(query_one(&self.0, sql))
    }
    fn pid(&self) -> i64 {
        let Value::Int(pid) = self.row("SELECT pg_backend_pid() AS pid")["pid"] else {
            panic!("expected pid")
        };
        pid
    }
    fn id(&self) -> i64 {
        let Value::Map(handle) = &self.0 else {
            panic!("expected handle")
        };
        let Value::Int(id) = handle["_pg_connection_id"] else {
            panic!("expected id")
        };
        id
    }
    fn execute(&self, sql: &str) {
        ok(call(
            "execute",
            &[
                self.0.clone(),
                Value::String(sql.into()),
                Value::Array(vec![]),
            ],
        ));
    }
    fn begin(&self) {
        ok(call("begin", std::slice::from_ref(&self.0)));
    }
    fn commit(&self) {
        ok(call("commit", std::slice::from_ref(&self.0)));
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        call("close", std::slice::from_ref(&self.0));
    }
}
fn query_one(handle: &Value, sql: &str) -> Value {
    call(
        "query_one",
        &[
            handle.clone(),
            Value::String(sql.into()),
            Value::Array(vec![]),
        ],
    )
}
fn row(value: Value) -> std::collections::HashMap<String, Value> {
    let value = ok(value);
    let Value::EnumValue {
        enum_name,
        variant,
        mut values,
    } = value
    else {
        panic!("expected Some row")
    };
    assert_eq!((enum_name.as_str(), variant.as_str()), ("Option", "Some"));
    let Value::Map(row) = values.remove(0) else {
        panic!("expected row map")
    };
    row
}
struct Fixture {
    url: reqwest::Url,
    prefix: String,
    monitor: postgres::Client,
}
impl Fixture {
    fn new() -> Self {
        let raw = std::env::var("NTNT_POSTGRES_TEST_URL")
            .expect("set a disposable NTNT_POSTGRES_TEST_URL");
        Self {
            url: reqwest::Url::parse(&raw).unwrap(),
            prefix: format!("ntnt142{}", uuid::Uuid::new_v4().simple()),
            monitor: postgres::Client::connect(&raw, postgres::NoTls)
                .expect("disposable Postgres must connect"),
        }
    }
    fn name(&self, slot: &str) -> String {
        format!("{}_{}", self.prefix, slot)
    }
    fn url(&self, slot: &str) -> String {
        let mut url = self.url.clone();
        url.query_pairs_mut()
            .append_pair("application_name", &self.name(slot));
        url.into()
    }
    fn active_names(&mut self) -> Vec<String> {
        let pattern = format!("{}%", self.prefix);
        let mut names: Vec<String> = self
            .monitor
            .query(
                "SELECT application_name FROM pg_stat_activity WHERE application_name LIKE $1",
                &[&pattern],
            )
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect();
        names.sort();
        names
    }
    fn wait_names(&mut self, slots: &[&str]) {
        let mut expected: Vec<_> = slots.iter().map(|slot| self.name(slot)).collect();
        expected.sort();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let names = self.active_names();
            if names == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "connections did not settle: expected {expected:?}, got {names:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn wait_at_most(&mut self, limit: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let names = self.active_names();
            if names.len() <= limit {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "physical connections exceeded cap: {names:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn wait_locked(&mut self, slot: &str) {
        let name = self.name(slot);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let blocked: bool = self.monitor.query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock')", &[&name]
            ).unwrap().get(0);
            if blocked {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "query never reached its lock barrier"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn isolated(name: &str, cap: &str, scenario: fn()) {
    std::env::var("NTNT_POSTGRES_TEST_URL").expect("set a disposable NTNT_POSTGRES_TEST_URL");
    if std::env::var("NTNT_POOL_LIFECYCLE_CHILD").as_deref() == Ok(name) {
        scenario();
        return;
    }
    let mut output = tempfile::tempfile().unwrap();
    let mut child = Process(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                name,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("NTNT_POOL_LIFECYCLE_CHILD", name)
            .env("NTNT_POSTGRES_MAX_SHARED_POOLS", cap)
            .env("NTNT_DB_POOL_SIZE", "1")
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output.try_clone().unwrap()))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "pool lifecycle child timed out");
        std::thread::sleep(Duration::from_millis(10));
    };
    output.rewind().unwrap();
    let mut text = String::new();
    output.read_to_string(&mut text).unwrap();
    assert!(status.success(), "{name}: {text}");
}
fn expect_capacity_error(url: &str) {
    let message = error(call("connect", &[Value::String(url.into())]));
    assert!(
        message.contains("NTNT_POSTGRES_MAX_SHARED_POOLS"),
        "{message}"
    );
    assert!(
        !message.contains(url),
        "capacity error must not echo connection URL"
    );
}

#[test]
#[ignore = "requires disposable NTNT_POSTGRES_TEST_URL"]
fn postgres_pool_lifecycle_reuse_lru_and_churn() {
    isolated("postgres_pool_lifecycle_reuse_lru_and_churn", "2", || {
        let mut f = Fixture::new();
        let a = Db::open(&f.url("a"));
        let a_pid = a.pid();
        let a_id = a.id();
        drop(a);
        let b = Db::open(&f.url("b"));
        let b_pid = b.pid();
        drop(b);
        let a = Db::open(&f.url("a"));
        assert_eq!(a.pid(), a_pid);
        assert_ne!(a.id(), a_id, "reuse must mint a fresh logical handle");
        drop(a);
        drop(Db::open(&f.url("c")));
        f.wait_names(&["a", "c"]);
        let a = Db::open(&f.url("a"));
        assert_eq!(a.pid(), a_pid, "MRU pool should remain reusable");
        let b = Db::open(&f.url("b"));
        assert_ne!(b.pid(), b_pid, "evicted pool must create a new backend");
        f.wait_names(&["a", "b"]);
        drop(a);
        drop(b);
        for n in 0..12 {
            let db = Db::open(&f.url(&format!("churn{n}")));
            assert!(db.pid() > 0);
            drop(db);
            f.wait_at_most(2);
        }
    });
}

#[test]
#[ignore = "requires disposable NTNT_POSTGRES_TEST_URL"]
fn postgres_pool_lifecycle_live_handles_and_transactions() {
    isolated(
        "postgres_pool_lifecycle_live_handles_and_transactions",
        "2",
        || {
            let mut f = Fixture::new();
            let a = Db::open(&f.url("a"));
            let b = Db::open(&f.url("b"));
            let a_pid = a.pid();
            let b_pid = b.pid();
            expect_capacity_error(&f.url("c"));
            assert_eq!(a.pid(), a_pid);
            assert_eq!(b.pid(), b_pid);
            a.begin();
            a.execute("CREATE TEMP TABLE pool_tx_probe (value INTEGER) ON COMMIT DROP");
            a.execute("INSERT INTO pool_tx_probe VALUES (42)");
            drop(b);
            let c = Db::open(&f.url("c"));
            assert!(matches!(
                a.row("SELECT value FROM pool_tx_probe")["value"],
                Value::Int(42)
            ));
            assert_eq!(a.pid(), a_pid, "transaction's client must stay pinned");
            expect_capacity_error(&f.url("d"));
            a.commit();
            assert_eq!(a.pid(), a_pid);
            f.wait_names(&["a", "c"]);
            drop(c);
            drop(a);
        },
    );
}

#[test]
#[ignore = "requires disposable NTNT_POSTGRES_TEST_URL"]
fn postgres_pool_lifecycle_inflight_close_keeps_pool_pinned() {
    isolated(
        "postgres_pool_lifecycle_inflight_close_keeps_pool_pinned",
        "1",
        || {
            let mut f = Fixture::new();
            for transaction in [false, true] {
                let slot = if transaction { "txn" } else { "query" };
                let db = Db::open(&f.url(slot));
                if transaction {
                    db.begin();
                }
                let key = uuid::Uuid::new_v4().as_u128() as i64;
                f.monitor
                    .query_one("SELECT pg_advisory_lock($1)", &[&key])
                    .unwrap();
                // Value can contain non-Send interpreter closures. Transport only
                // the already-issued opaque handle fields, not a Value itself.
                let id = db.id();
                let Value::Map(fields) = &db.0 else {
                    unreachable!()
                };
                let Value::String(token) = &fields["_pg_handle_token"] else {
                    unreachable!()
                };
                let token = token.clone();
                let query = std::thread::spawn(move || {
                    let handle = Value::Map(std::collections::HashMap::from([
                        ("_pg_connection_id".into(), Value::Int(id)),
                        ("_pg_handle_token".into(), Value::String(token)),
                        ("connected".into(), Value::Bool(true)),
                    ]));
                    row(query_one(
                        &handle,
                        &format!("SELECT pg_advisory_xact_lock({key}::bigint) AS locked"),
                    ));
                });
                f.wait_locked(slot);
                drop(db);
                expect_capacity_error(&f.url("replacement"));
                f.monitor
                    .query_one("SELECT pg_advisory_unlock($1)", &[&key])
                    .unwrap();
                query.join().unwrap();
                let replacement = Db::open(&f.url("replacement"));
                assert!(replacement.pid() > 0);
                f.wait_names(&["replacement"]);
                drop(replacement);
            }
        },
    );
}

#[test]
#[ignore = "requires disposable NTNT_POSTGRES_TEST_URL"]
fn postgres_pool_lifecycle_concurrent_admission() {
    isolated("postgres_pool_lifecycle_concurrent_admission", "2", || {
        let mut f = Fixture::new();
        for same_key in [false, true] {
            let start = Arc::new(Barrier::new(9));
            let release = Arc::new(Barrier::new(9));
            let (send, recv) = mpsc::channel();
            let mut threads = Vec::new();
            for n in 0..8 {
                let url = f.url(&if same_key {
                    "shared".into()
                } else {
                    format!("concurrent{n}")
                });
                let start = start.clone();
                let release = release.clone();
                let send = send.clone();
                threads.push(std::thread::spawn(move || {
                    start.wait();
                    let result = call("connect", &[Value::String(url)]);
                    let db = match &result {
                        Value::EnumValue { variant, .. } if variant == "Ok" => Some(Db(ok(result))),
                        _ => {
                            let message = error(result);
                            assert!(
                                message.contains("NTNT_POSTGRES_MAX_SHARED_POOLS"),
                                "{message}"
                            );
                            None
                        }
                    };
                    send.send(db.as_ref().map(|db| (db.id(), db.pid())))
                        .unwrap();
                    release.wait();
                    drop(db);
                }));
            }
            start.wait();
            let results: Vec<_> = (0..8)
                .map(|_| recv.recv_timeout(Duration::from_secs(10)).unwrap())
                .collect();
            let successes: Vec<_> = results.into_iter().flatten().collect();
            assert_eq!(successes.len(), if same_key { 8 } else { 2 });
            assert_eq!(
                successes
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<HashSet<_>>()
                    .len(),
                successes.len()
            );
            if same_key {
                assert_eq!(
                    successes
                        .iter()
                        .map(|(_, pid)| *pid)
                        .collect::<HashSet<_>>()
                        .len(),
                    1
                );
            }
            f.wait_at_most(2);
            release.wait();
            for thread in threads {
                thread.join().unwrap();
            }
        }
    });
}

#[test]
#[ignore = "requires disposable NTNT_POSTGRES_TEST_URL"]
fn postgres_pool_lifecycle_failed_creation_releases_capacity() {
    isolated(
        "postgres_pool_lifecycle_failed_creation_releases_capacity",
        "1",
        || {
            let mut f = Fixture::new();
            for _ in 0..3 {
                let mut invalid = reqwest::Url::parse(&f.url("failed")).unwrap();
                invalid
                    .set_password(Some("ntnt-pool-secret-canary"))
                    .unwrap();
                let message = error(call("connect", &[Value::String(invalid.to_string())]));
                assert!(!message.contains("ntnt-pool-secret-canary"));
                assert!(!message.contains(invalid.as_str()));
                let valid = Db::open(&f.url("valid"));
                assert!(valid.pid() > 0);
                drop(valid);
                f.wait_names(&["valid"]);
            }
        },
    );
}
