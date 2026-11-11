//! Run explicitly against a disposable superuser fixture:
//! NTNT_AUTH_TEST_POSTGRES_URL=postgres://... cargo test --test auth_postgres_runtime_tests -- --ignored
//! Creates isolated databases and distinct owner/runtime roles; never use a live database.
use ntnt::stdlib::auth::{ensure_auth_session_store, AuthConfig, SessionStore};
use postgres::{Client, NoTls};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture {
    admin: Client,
    owner: String,
    runtime: String,
    database: String,
    owner_url: String,
    runtime_url: String,
}
impl Fixture {
    fn new() -> Self {
        let raw = std::env::var("NTNT_AUTH_TEST_POSTGRES_URL")
            .expect("set NTNT_AUTH_TEST_POSTGRES_URL to a disposable superuser fixture");
        let mut admin = Client::connect(&raw, NoTls).unwrap();
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let owner = format!("app_owner_{suffix}");
        let runtime = format!("app_runtime_{suffix}");
        let database = format!("auth_{suffix}");
        admin.batch_execute(&format!(
            "CREATE ROLE {owner} LOGIN PASSWORD 'ntnt_auth_fixture' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
             CREATE ROLE {runtime} LOGIN PASSWORD 'ntnt_auth_fixture' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;"
        )).unwrap();
        admin
            .batch_execute(&format!("CREATE DATABASE {database} OWNER {owner}"))
            .unwrap();
        let url = |user: &str| {
            let mut url = reqwest::Url::parse(&raw).unwrap();
            url.set_username(user).unwrap();
            url.set_password(Some("ntnt_auth_fixture")).unwrap();
            url.set_path(&database);
            // Public fixture-only password, never an application credential.
            url.to_string()
        };
        let owner_url = url(&owner);
        let runtime_url = url(&runtime);
        let mut f = Self {
            admin,
            owner,
            runtime,
            database,
            owner_url,
            runtime_url,
        };
        f.owner()
            .batch_execute("REVOKE CREATE ON SCHEMA public FROM PUBLIC")
            .unwrap();
        f
    }
    fn owner(&mut self) -> Client {
        Client::connect(&self.owner_url, NoTls).unwrap()
    }
    fn initialize(url: &str) -> Result<(), String> {
        ensure_auth_session_store(&AuthConfig {
            session_secret: "fixture-only-session-secret".into(),
            session_store: SessionStore::Postgres(url.into()),
            ..AuthConfig::default()
        })
    }
    fn bootstrap(&mut self) {
        Self::initialize(&self.owner_url).expect("owner bootstrap");
        self.grant_runtime();
    }
    fn grant_runtime(&mut self) {
        self.grant_runtime_in("public");
    }
    fn grant_runtime_in(&mut self, schema: &str) {
        self.owner()
            .batch_execute(&format!(
                "GRANT CONNECT ON DATABASE {} TO {};
             GRANT USAGE ON SCHEMA {schema} TO {};
             GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO {};",
                self.database, self.runtime, self.runtime, self.runtime
            ))
            .unwrap();
    }
    fn non_public() -> Self {
        let mut f = Self::new();
        f.owner()
            .batch_execute(&format!(
                "CREATE SCHEMA auth_first; CREATE SCHEMA auth_data;
             GRANT USAGE ON SCHEMA auth_first TO {}",
                f.runtime
            ))
            .unwrap();
        f.admin
            .batch_execute(&format!(
                "ALTER ROLE {} SET search_path=auth_data,public;
             ALTER ROLE {} SET search_path=auth_first,auth_data,public",
                f.owner, f.runtime
            ))
            .unwrap();
        Self::initialize(&f.owner_url).unwrap();
        f.grant_runtime_in("auth_data");
        let counts = f.owner().query_one(
            "SELECT (SELECT count(*) FROM pg_class WHERE relnamespace='auth_data'::regnamespace AND relkind='r'),
                    (SELECT count(*) FROM pg_class WHERE relnamespace='public'::regnamespace AND relkind='r')", &[]
        ).unwrap();
        assert_eq!((counts.get::<_, i64>(0), counts.get::<_, i64>(1)), (8, 0));
        let role = Client::connect(&f.runtime_url, NoTls).unwrap().query_one(
            "SELECT current_schema()::text, has_schema_privilege(current_user, 'auth_first', 'CREATE'),
                    has_schema_privilege(current_user, 'auth_data', 'CREATE'),
                    pg_has_role(current_user, $1, 'MEMBER'), rolsuper, rolcreatedb, rolcreaterole,
                    rolreplication, rolbypassrls FROM pg_roles WHERE rolname=current_user", &[&f.owner]
        ).unwrap();
        assert_eq!(role.get::<_, String>(0), "auth_first");
        for column in 1..9 {
            assert!(
                !role.get::<_, bool>(column),
                "runtime unexpectedly privileged"
            );
        }
        f
    }
    fn snapshot(&mut self) -> String {
        // Includes catalog tuple versions: even a no-op ALTER or ACL write must be detected.
        let mut client = self.owner();
        let schema: String = client.query_one(
            "SELECT jsonb_build_object(
                'relations', (SELECT jsonb_agg(row_to_json(c) ORDER BY c.oid) FROM
                    (SELECT oid, relname, relowner, relacl, xmin::text FROM pg_class
                     WHERE relnamespace = 'public'::regnamespace) c),
                'columns', (SELECT jsonb_agg(row_to_json(a) ORDER BY a.attrelid, a.attnum) FROM
                    (SELECT attrelid, attnum, attname, atttypid, attnotnull, xmin::text FROM pg_attribute
                     WHERE attrelid IN (SELECT oid FROM pg_class WHERE relnamespace = 'public'::regnamespace)) a),
                'constraints', (SELECT jsonb_agg(row_to_json(c) ORDER BY c.oid) FROM
                    (SELECT oid, conrelid, pg_get_constraintdef(oid), xmin::text FROM pg_constraint
                     WHERE connamespace = 'public'::regnamespace) c),
                'namespace', (SELECT row_to_json(n) FROM (SELECT nspowner, nspacl, xmin::text FROM pg_namespace WHERE oid='public'::regnamespace) n),
                'defaults', (SELECT jsonb_agg(row_to_json(d) ORDER BY d.oid) FROM
                    (SELECT oid, adrelid, adnum, pg_get_expr(adbin, adrelid), xmin::text FROM pg_attrdef
                     WHERE adrelid IN (SELECT oid FROM pg_class WHERE relnamespace='public'::regnamespace)) d),
                'records', (SELECT jsonb_agg(row_to_json(t)) FROM auth_sessions t)
             )::text", &[]
        ).unwrap().get(0);
        let mut records = Vec::new();
        for table in [
            "auth_sessions",
            "auth_oauth_states",
            "auth_exchange_tokens",
            "auth_challenges",
            "auth_rate_limits",
            "auth_local_identities",
            "auth_local_credentials",
            "auth_local_one_time_tokens",
        ] {
            let exists: bool = client
                .query_one("SELECT to_regclass($1) IS NOT NULL", &[&table])
                .unwrap()
                .get(0);
            let data: Option<String> = exists.then(|| client.query_one(&format!(
                "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), '[]'::jsonb)::text FROM {table} t"
            ), &[]).unwrap().get(0));
            records.push(data);
        }
        serde_json::to_string(&(schema, records)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.admin
            .batch_execute(&format!("DROP DATABASE {} WITH (FORCE)", self.database))
            .unwrap();
        self.admin
            .batch_execute(&format!(
                "DROP ROLE {}; DROP ROLE {};",
                self.runtime, self.owner
            ))
            .unwrap();
    }
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_missing_and_legacy_schema_requires_bootstrap_authority() {
    let mut empty = Fixture::new();
    empty.grant_runtime();
    let error = Fixture::initialize(&empty.runtime_url)
        .expect_err("empty runtime store must not use memory");
    assert!(
        error.contains("auth_sessions")
            && error.contains("42501")
            && error.contains("schema owner"),
        "{error}"
    );
    assert!(empty
        .owner()
        .query_one("SELECT to_regclass('auth_sessions') IS NULL", &[])
        .unwrap()
        .get::<_, bool>(0));
    empty.bootstrap();
    Fixture::initialize(&empty.runtime_url).unwrap();

    // Every supported additive migration family, plus missing table and index.
    for sql in [
        "ALTER TABLE auth_sessions DROP COLUMN device_name, DROP COLUMN user_agent_hash, DROP COLUMN last_ip_hash",
        "ALTER TABLE auth_oauth_states DROP COLUMN remember_me, DROP COLUMN device_name, DROP COLUMN user_agent_hash, DROP COLUMN last_ip_hash",
        "ALTER TABLE auth_challenges DROP COLUMN csrf_token",
        "DROP TABLE auth_exchange_tokens",
        "DROP INDEX idx_auth_local_one_time_tokens_one_magic_link",
    ] {
        let mut f = Fixture::new();
        f.bootstrap();
        f.owner().batch_execute(sql).unwrap();
        let before = f.snapshot();
        let error = Fixture::initialize(&f.runtime_url).expect_err("runtime must not migrate owner schema");
        assert!(error.contains("42501") && error.contains("schema owner"), "{error}");
        assert_eq!(f.snapshot(), before, "denied migration changed persistent state");
        Fixture::initialize(&f.owner_url).expect("supported owner migration");
        f.grant_runtime();
        Fixture::initialize(&f.runtime_url).expect("runtime after owner upgrade");
    }
}

fn auth_binary() -> PathBuf {
    std::env::var_os("NTNT_AUTH_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_ntnt")))
}

fn lint_program(binary: &Path, script: &Path) {
    let lint = Command::new(binary)
        .arg("lint")
        .arg(script)
        .output()
        .unwrap();
    assert!(
        lint.status.success(),
        "lint: {}{}",
        String::from_utf8_lossy(&lint.stdout),
        String::from_utf8_lossy(&lint.stderr)
    );
}

fn run_auth_program(url: &str, body: &str, variables: &[(&str, &str)]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("auth.tnt");
    std::fs::write(&script, format!(r#"
import {{ enable_auth }} from "std/auth"
import {{ get_env }} from "std/env"
enable_auth([], map {{ "session_store": unwrap(get_env("AUTH_STORE")), "session_secret": "fixture-only-session-secret", "cookie_secure": false }})
{body}
"#)).unwrap();
    let binary = auth_binary();
    lint_program(&binary, &script);
    let mut command = Command::new(binary);
    command
        .arg("run")
        .arg(&script)
        .env("AUTH_STORE", url)
        .env("NTNT_ENV", "production")
        .env("NTNT_TYPE_MODE", "strict");
    for &(key, value) in variables {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "auth program: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

struct Server(std::process::Child);
impl Server {
    fn start(
        binary: &Path,
        script: &Path,
        url: &str,
        log: &Path,
        http: &reqwest::blocking::Client,
        base: &str,
    ) -> Self {
        let stdout = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        let mut server = Self(
            Command::new(binary)
                .arg("run")
                .arg(script)
                .args(["--workers", "1"])
                .env("AUTH_STORE", url)
                .env("NTNT_ENV", "production")
                .env("NTNT_TYPE_MODE", "strict")
                .stdout(Stdio::from(stdout.try_clone().unwrap()))
                .stderr(Stdio::from(stdout))
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "server exited during enable_auth: {}",
                std::fs::read_to_string(log).unwrap()
            );
            if let Ok(response) = http.get(format!("{base}/ready")).send() {
                if response.status().is_success() && response.text().unwrap() == "ready" {
                    return server;
                }
            }
            assert!(
                Instant::now() < deadline,
                "server readiness timed out: {}",
                std::fs::read_to_string(log).unwrap()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn check_server_session_lifecycle(f: &mut Fixture) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("server.tnt");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let source = format!(
        r##"
import {{ enable_auth, sign_in_session, get_session, sign_out_session }} from "std/auth"
import {{ get_env }} from "std/env"
import {{ text }} from "std/http/server"
enable_auth([], map {{ "session_store": unwrap(get_env("AUTH_STORE")), "session_secret": "fixture-only-session-secret", "cookie_secure": false }})
get("/ready", fn(req) {{ return text("ready") }})
get("/signin", fn(req) {{ return sign_in_session(text("signed-in"), req, map {{ "subject_id": "runtime-user", "provider": "local", "email": "runtime@example.test", "data": map {{ "marker": "persisted" }} }}) }})
get("/session", fn(req) {{
    let session = get_session(req) otherwise {{ return text("none") }}
    return text("#{{session.user.id}}|#{{session.user.email}}|#{{session.data.marker}}")
}})
get("/logout", fn(req) {{ return sign_out_session(text("signed-out"), req) }})
listen({port})
"##
    );
    std::fs::write(&script, source).unwrap();
    let binary = auth_binary();
    lint_program(&binary, &script);
    let log = dir.path().join("server.log");
    let http = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let server = Server::start(&binary, &script, &f.runtime_url, &log, &http, &base);
    let get = |path: &str| {
        http.get(format!("{base}{path}"))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .unwrap()
    };
    let expected = "local:runtime-user|runtime@example.test|persisted";
    assert_eq!(get("/session"), "none");
    assert_eq!(get("/signin"), "signed-in");
    assert_eq!(get("/session"), expected);
    let row = f.owner().query_one("SELECT id, user_id, email, data_json::jsonb->>'marker' FROM auth_sessions WHERE user_id='local:runtime-user'", &[]).unwrap();
    let session_id: String = row.get(0);
    assert_eq!(
        (
            row.get::<_, String>(1),
            row.get::<_, String>(2),
            row.get::<_, String>(3)
        ),
        (
            "local:runtime-user".into(),
            "runtime@example.test".into(),
            "persisted".into()
        )
    );
    drop(server);
    let server = Server::start(&binary, &script, &f.runtime_url, &log, &http, &base);
    assert_eq!(
        get("/session"),
        expected,
        "payload must survive process cold-start, not memory fallback"
    );
    assert_eq!(
        f.owner()
            .query_one(
                "SELECT id FROM auth_sessions WHERE user_id='local:runtime-user'",
                &[]
            )
            .unwrap()
            .get::<_, String>(0),
        session_id
    );
    assert_eq!(get("/logout"), "signed-out");
    assert_eq!(get("/session"), "none");
    assert_eq!(
        f.owner()
            .query_one(
                "SELECT count(*) FROM auth_sessions WHERE user_id='local:runtime-user'",
                &[]
            )
            .unwrap()
            .get::<_, i64>(0),
        0,
        "logout must delete persisted session"
    );
    drop(server);
    assert!(!std::fs::read_to_string(log)
        .unwrap()
        .contains("Falling back to memory"));
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_dml_runtime_server_sign_in_lookup_persist_logout() {
    let mut f = Fixture::new();
    f.bootstrap();
    check_server_session_lifecycle(&mut f);
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_non_public_schema_runtime_preserves_session_across_restart() {
    let mut f = Fixture::non_public();
    f.admin
        .batch_execute(&format!(
            "ALTER ROLE {} SET default_transaction_read_only=on",
            f.runtime
        ))
        .unwrap();
    Fixture::initialize(&f.runtime_url).expect("read-only startup in later non-public schema");
    f.admin
        .batch_execute(&format!(
            "ALTER ROLE {} RESET default_transaction_read_only",
            f.runtime
        ))
        .unwrap();
    check_server_session_lifecycle(&mut f);
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_schema_validation_checks_constraint_enforcement_and_index_table() {
    let mut f = Fixture::non_public();
    for (mutation, repair, object) in [
        ("ALTER TABLE auth_local_credentials DROP CONSTRAINT auth_local_credentials_local_user_id_fkey; ALTER TABLE auth_local_credentials ADD CONSTRAINT reviewed_fk FOREIGN KEY(local_user_id) REFERENCES auth_local_identities(id) ON DELETE CASCADE NOT VALID",
         "ALTER TABLE auth_local_credentials VALIDATE CONSTRAINT reviewed_fk", "required constraint"),
        ("ALTER TABLE auth_local_credentials ALTER CONSTRAINT reviewed_fk DEFERRABLE INITIALLY IMMEDIATE",
         "ALTER TABLE auth_local_credentials ALTER CONSTRAINT reviewed_fk NOT DEFERRABLE", "required constraint"),
        ("DROP INDEX idx_sessions_expires; CREATE TABLE index_decoy(expires_at bigint); CREATE INDEX idx_sessions_expires ON index_decoy(expires_at)",
         "DROP INDEX idx_sessions_expires; DROP TABLE index_decoy; CREATE INDEX idx_sessions_expires ON auth_sessions(expires_at)", "idx_sessions_expires"),
    ] {
        f.owner().batch_execute(mutation).unwrap();
        let error = Fixture::initialize(&f.runtime_url).expect_err("incompatible catalog shape must fail");
        assert!(error.contains("incompatible") && error.contains(object), "unexpected rejection: {error}");
        f.owner().batch_execute(repair).unwrap();
        Fixture::initialize(&f.runtime_url).expect("canonical repair must restore startup");
    }
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_legacy_token_failure_rolls_back_and_owner_can_migrate() {
    let mut f = Fixture::new();
    f.bootstrap();
    f.owner().batch_execute(
        "INSERT INTO auth_local_identities
         VALUES ('legacy-user', 'email', 'legacy@example.test', 'legacy@example.test', 1, 1, 'active', '{}')"
    ).unwrap();
    // Issue through the public API, then stage the same hashed record in the legacy layout.
    let output = run_auth_program(
        &f.runtime_url,
        r#"
import { issue_password_reset } from "std/auth"
import { stringify } from "std/json"
let issued = unwrap(issue_password_reset("legacy@example.test"))
print("ISSUED=#{stringify(issued)}")
"#,
        &[],
    );
    let issued: serde_json::Value = serde_json::from_str(
        output
            .lines()
            .find_map(|line| line.strip_prefix("ISSUED="))
            .expect("issued-token payload"),
    )
    .unwrap();
    assert_eq!(issued["status"], "accepted");
    let token = issued["token"].as_str().unwrap().to_string();
    let selector = issued["selector"].as_str().unwrap().to_string();
    let token_hash: String;
    {
        let mut owner = f.owner();
        token_hash = owner
            .query_one(
                "SELECT token_hash FROM auth_local_one_time_tokens WHERE selector=$1",
                &[&selector],
            )
            .unwrap()
            .get(0);
        owner.batch_execute(
            "CREATE TABLE auth_local_password_reset_tokens (
                 selector TEXT PRIMARY KEY, local_user_id TEXT NOT NULL REFERENCES auth_local_identities(id) ON DELETE CASCADE,
                 token_hash TEXT NOT NULL, created_at BIGINT NOT NULL, expires_at BIGINT NOT NULL)"
        ).unwrap();
        assert_eq!(owner.execute(
            "INSERT INTO auth_local_password_reset_tokens SELECT selector,local_user_id,token_hash,created_at,expires_at
             FROM auth_local_one_time_tokens WHERE selector=$1", &[&selector]
        ).unwrap(), 1, "issued reset record must exist; dummy token material cannot satisfy this test");
        assert_eq!(
            owner
                .execute(
                    "DELETE FROM auth_local_one_time_tokens WHERE selector=$1",
                    &[&selector]
                )
                .unwrap(),
            1
        );
    }
    f.grant_runtime();
    let error =
        Fixture::initialize(&f.runtime_url).expect_err("runtime cannot drop a legacy owner table");
    let counts = f.owner().query_one(
        "SELECT (SELECT count(*) FROM auth_local_password_reset_tokens), (SELECT count(*) FROM auth_local_one_time_tokens)", &[]
    ).unwrap();
    assert_eq!(
        counts.get::<_, i64>(0),
        1,
        "failed migration lost source token"
    );
    assert_eq!(
        counts.get::<_, i64>(1),
        0,
        "failed migration committed partial copy"
    );
    assert!(
        error.contains("schema owner") && error.contains("42501"),
        "unclear migration permission error: {error}"
    );
    // Force an owner-side failure *after* copy/delete; all DDL and data must roll back.
    f.owner()
        .batch_execute(
            "ALTER TABLE auth_sessions DROP COLUMN device_name;
         CREATE VIEW legacy_dependency AS SELECT selector FROM auth_local_password_reset_tokens",
        )
        .unwrap();
    let before = f.snapshot();
    let error =
        Fixture::initialize(&f.owner_url).expect_err("dependent legacy view must block DROP");
    assert!(error.contains("2BP01"), "unexpected failure: {error}");
    assert_eq!(
        f.snapshot(),
        before,
        "failed owner migration committed additive DDL"
    );
    let counts = f.owner().query_one(
        "SELECT (SELECT count(*) FROM auth_local_password_reset_tokens), (SELECT count(*) FROM auth_local_one_time_tokens)", &[]
    ).unwrap();
    assert_eq!((counts.get::<_, i64>(0), counts.get::<_, i64>(1)), (1, 0));
    f.owner()
        .batch_execute("DROP VIEW legacy_dependency")
        .unwrap();
    Fixture::initialize(&f.owner_url).unwrap();
    let row = f.owner().query_one(
        "SELECT purpose, local_user_id, token_hash FROM auth_local_one_time_tokens WHERE selector=$1", &[&selector]
    ).unwrap();
    assert_eq!(row.get::<_, String>(0), "password_reset");
    assert_eq!(row.get::<_, String>(1), "legacy-user");
    assert_eq!(row.get::<_, String>(2), token_hash);
    assert!(f
        .owner()
        .query_one(
            "SELECT to_regclass('auth_local_password_reset_tokens') IS NULL",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
    let consumed = run_auth_program(
        &f.runtime_url,
        r#"
import { consume_password_reset, verify_local_password } from "std/auth"
let consumed = unwrap(consume_password_reset(unwrap(get_env("RESET_TOKEN")), "rotated fixture password"))
assert(consumed.local_user_id == "legacy-user")
let verified = unwrap(verify_local_password("legacy@example.test", "rotated fixture password"))
assert(verified.local_user_id == "legacy-user")
print("CONSUMED")
"#,
        &[("RESET_TOKEN", token.as_str())],
    );
    assert!(consumed.lines().any(|line| line == "CONSUMED"));
    assert_eq!(
        f.owner()
            .query_one("SELECT count(*) FROM auth_local_one_time_tokens", &[])
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    Fixture::initialize(&f.runtime_url).unwrap();
    let replay = run_auth_program(
        &f.runtime_url,
        r#"
import { consume_password_reset, verify_local_password } from "std/auth"
match consume_password_reset(unwrap(get_env("RESET_TOKEN")), "replayed fixture password") {
    Err(message) => assert(message == "Invalid password reset token"),
    Ok(_) => assert(false)
}
let verified = unwrap(verify_local_password("legacy@example.test", "rotated fixture password"))
assert(verified.local_user_id == "legacy-user")
print("REPLAY_REJECTED")
"#,
        &[("RESET_TOKEN", token.as_str())],
    );
    assert!(replay.lines().any(|line| line == "REPLAY_REJECTED"));
    assert_eq!(
        f.owner()
            .query_one("SELECT count(*) FROM auth_local_one_time_tokens", &[])
            .unwrap()
            .get::<_, i64>(0),
        0,
        "consumed legacy token replayed"
    );
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_rejects_existing_but_incompatible_schema() {
    let mut accepted = Vec::new();
    for sql in [
        "ALTER TABLE auth_sessions DROP COLUMN email",
        "ALTER TABLE auth_sessions ALTER COLUMN email TYPE integer USING NULL::integer",
        "ALTER TABLE auth_sessions ALTER COLUMN user_id DROP NOT NULL",
        "ALTER TABLE auth_sessions DROP CONSTRAINT auth_sessions_pkey",
        "ALTER TABLE auth_local_identities DROP CONSTRAINT auth_local_identities_identifier_kind_identifier_normalized_key",
        "ALTER TABLE auth_local_credentials DROP CONSTRAINT auth_local_credentials_local_user_id_fkey",
        "ALTER TABLE auth_local_one_time_tokens DROP CONSTRAINT auth_local_one_time_tokens_purpose_check",
        "ALTER TABLE auth_challenges ALTER COLUMN csrf_token DROP DEFAULT",
        "DROP INDEX idx_auth_local_one_time_tokens_one_magic_link; CREATE INDEX idx_auth_local_one_time_tokens_one_magic_link ON auth_local_one_time_tokens(local_user_id)",
        "DROP TABLE auth_exchange_tokens; CREATE VIEW auth_exchange_tokens AS SELECT ''::text AS token, ''::text AS session_id, 0::bigint AS created_at",
    ] {
        let mut f = Fixture::new();
        f.bootstrap();
        f.owner().batch_execute(sql).unwrap();
        match Fixture::initialize(&f.owner_url) {
            Ok(()) => accepted.push(sql),
            Err(message) => assert!(message.contains("incompatible"), "unclear schema error: {message}"),
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted incompatible schemas: {accepted:#?}"
    );
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_current_schema_accepts_dml_runtime_without_mutation() {
    let mut f = Fixture::new();
    f.bootstrap();
    // Tables can resolve later in search_path than current_schema().
    f.owner()
        .batch_execute(&format!(
            "CREATE SCHEMA auth_first; GRANT USAGE ON SCHEMA auth_first TO {}",
            f.runtime
        ))
        .unwrap();
    f.admin
        .batch_execute(&format!(
            "ALTER ROLE {} SET search_path=auth_first,public",
            f.runtime
        ))
        .unwrap();
    let row = Client::connect(&f.runtime_url, NoTls)
        .unwrap()
        .query_one(
            "SELECT has_schema_privilege(current_user, 'public', 'CREATE'),
                pg_has_role(current_user, $1, 'MEMBER'), rolsuper, rolcreatedb, rolcreaterole,
                rolreplication, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[&f.owner],
        )
        .unwrap();
    for n in 0..7 {
        assert!(!row.get::<_, bool>(n), "runtime unexpectedly privileged");
    }
    f.owner().batch_execute(
        "INSERT INTO auth_sessions (id,user_id,provider,raw_json,data_json,created_at,expires_at)
         VALUES ('existing','existing-user','local','{}','{}',1,9223372036854775807);
         INSERT INTO auth_oauth_states (state,provider,redirect_url,created_at) VALUES ('existing-state','local','/after',1);
         INSERT INTO auth_exchange_tokens VALUES ('existing-exchange','existing',1);
         INSERT INTO auth_challenges (id,subject_id,provider,kind,data_json,created_at,expires_at)
             VALUES ('existing-challenge','existing-user','local','login','{}',1,9223372036854775807);
         INSERT INTO auth_rate_limits VALUES ('login','existing-key',1,9223372036854775807);
         INSERT INTO auth_local_identities VALUES ('existing-user','email','existing@example.test','existing@example.test',1,1,'active','{}');
         INSERT INTO auth_local_credentials (local_user_id,password_hash,password_hash_algorithm,password_hash_params_json,password_changed_at)
             VALUES ('existing-user','fixture-hash','fixture-algorithm','{}',1);
         INSERT INTO auth_local_one_time_tokens VALUES ('password_reset','existing-selector','existing-user','fixture-hash',1,9223372036854775807)"
    ).unwrap();
    let before = f.snapshot();
    Fixture::initialize(&f.runtime_url).expect("DML-only cold-start of a current store");
    assert_eq!(
        f.snapshot(),
        before,
        "startup changed schema, ownership, ACLs or existing data"
    );
    f.admin
        .batch_execute(&format!(
            "ALTER ROLE {} SET default_transaction_read_only=on",
            f.runtime
        ))
        .unwrap();
    Fixture::initialize(&f.runtime_url)
        .expect("current startup must work in a read-only transaction");
    Fixture::initialize(&f.owner_url).unwrap();
    assert_eq!(
        f.snapshot(),
        before,
        "current owner startup must also be read-only"
    );
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_initialization_fails_instead_of_hanging_on_a_held_lock() {
    // A stalled process holding the initialization lock must make startup
    // fail with a lock-timeout error, not wait forever.
    let mut f = Fixture::new();
    f.bootstrap();
    let mut holder = f.owner();
    let mut held = holder.transaction().unwrap();
    held.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended('ntnt:auth:local-one-time-token-migration', 0))",
        &[],
    )
    .unwrap();
    let url = f.runtime_url.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(Fixture::initialize(&url));
    });
    let started = Instant::now();
    let result = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("initialization still waiting on the lock after 60s");
    let error = result.expect_err("initialization must fail while the lock is held");
    assert!(
        error.contains("lock initialization") && error.contains("55P03"),
        "{error}"
    );
    assert!(
        started.elapsed() >= Duration::from_secs(25),
        "{:?}",
        started.elapsed()
    );
    held.rollback().unwrap();
    Fixture::initialize(&f.runtime_url).expect("initialization once the lock is free");
}
