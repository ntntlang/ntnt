//! Run explicitly against a disposable superuser fixture:
//! NTNT_AUTH_TEST_POSTGRES_URL=postgres://... cargo test --test auth_postgres_runtime_tests -- --ignored
//! Creates isolated databases and distinct owner/runtime roles; never use a live database.
use ntnt::stdlib::auth::{ensure_auth_session_store, AuthConfig, SessionStore};
use postgres::{Client, NoTls};

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
        self.owner()
            .batch_execute(&format!(
                "GRANT CONNECT ON DATABASE {} TO {};
             GRANT USAGE ON SCHEMA public TO {};
             GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO {};",
                self.database, self.runtime, self.runtime, self.runtime
            ))
            .unwrap();
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

struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires disposable NTNT_AUTH_TEST_POSTGRES_URL (creates databases and roles)"]
fn postgres_dml_runtime_server_sign_in_lookup_persist_logout() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut f = Fixture::new();
    f.bootstrap();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("server.tnt");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let source = format!(
        r#"
import {{ enable_auth, sign_in_session, get_session, sign_out_session }} from "std/auth"
import {{ get_env }} from "std/env"
import {{ text }} from "std/http/server"
enable_auth([], map {{ "session_store": unwrap(get_env("AUTH_STORE")), "session_secret": "fixture-only-session-secret", "cookie_secure": false }})
get("/ready", fn(req) {{ return text("ready") }})
get("/signin", fn(req) {{ return sign_in_session(text("signed-in"), req, map {{ "subject_id": "runtime-user", "provider": "local" }}) }})
get("/session", fn(req) {{
    let session = get_session(req) otherwise {{ return text("none") }}
    return text("present")
}})
get("/logout", fn(req) {{ return sign_out_session(text("signed-out"), req) }})
listen({port})
"#
    );
    std::fs::write(&script, source).unwrap();
    let binary = std::env::var_os("NTNT_AUTH_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_ntnt")));
    let lint = Command::new(&binary)
        .arg("lint")
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        lint.status.success(),
        "lint: {}{}",
        String::from_utf8_lossy(&lint.stdout),
        String::from_utf8_lossy(&lint.stderr)
    );
    let log = dir.path().join("server.log");
    let stdout = std::fs::File::create(&log).unwrap();
    let mut server = Server(
        Command::new(&binary)
            .arg("run")
            .arg(&script)
            .env("AUTH_STORE", &f.runtime_url)
            .env("NTNT_ENV", "production")
            .env("NTNT_TYPE_MODE", "strict")
            .env("NTNT_HTTP_WORKERS", "1")
            .stdout(Stdio::from(stdout.try_clone().unwrap()))
            .stderr(Stdio::from(stdout))
            .spawn()
            .unwrap(),
    );
    let http = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(response) = http.get(format!("{base}/ready")).send() {
            if response.status().is_success() {
                break;
            }
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "server exited during enable_auth: {}",
            std::fs::read_to_string(&log).unwrap()
        );
        assert!(
            Instant::now() < deadline,
            "server readiness timed out: {}",
            std::fs::read_to_string(&log).unwrap()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let get = |path: &str| {
        http.get(format!("{base}{path}"))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .unwrap()
    };
    assert_eq!(get("/session"), "none");
    assert_eq!(get("/signin"), "signed-in");
    assert_eq!(get("/session"), "present");
    let count = |client: &mut Client| {
        client
            .query_one(
                "SELECT count(*) FROM auth_sessions WHERE user_id='local:runtime-user'",
                &[],
            )
            .unwrap()
            .get::<_, i64>(0)
    };
    assert_eq!(
        count(&mut f.owner()),
        1,
        "session must persist in PostgreSQL, not memory fallback"
    );
    assert_eq!(get("/logout"), "signed-out");
    assert_eq!(get("/session"), "none");
    assert_eq!(
        count(&mut f.owner()),
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
fn postgres_legacy_token_failure_rolls_back_and_owner_can_migrate() {
    let mut f = Fixture::new();
    f.bootstrap();
    f.owner().batch_execute(
        "INSERT INTO auth_local_identities
         VALUES ('legacy-user', 'email', 'legacy@example.test', 'legacy@example.test', 1, 1, 'active', '{}');
         CREATE TABLE auth_local_password_reset_tokens (
             selector TEXT PRIMARY KEY, local_user_id TEXT NOT NULL REFERENCES auth_local_identities(id) ON DELETE CASCADE,
             token_hash TEXT NOT NULL, created_at BIGINT NOT NULL, expires_at BIGINT NOT NULL);
         INSERT INTO auth_local_password_reset_tokens VALUES ('legacy-token', 'legacy-user', 'fixture-hash', 1, 9999999999);"
    ).unwrap();
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
        "SELECT purpose, local_user_id, token_hash FROM auth_local_one_time_tokens WHERE selector='legacy-token'", &[]
    ).unwrap();
    assert_eq!(row.get::<_, String>(0), "password_reset");
    assert_eq!(row.get::<_, String>(1), "legacy-user");
    assert_eq!(row.get::<_, String>(2), "fixture-hash");
    assert!(f
        .owner()
        .query_one(
            "SELECT to_regclass('auth_local_password_reset_tokens') IS NULL",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
    f.owner()
        .batch_execute("DELETE FROM auth_local_one_time_tokens")
        .unwrap();
    Fixture::initialize(&f.runtime_url).unwrap();
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
