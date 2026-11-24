//! Dedicated physical PostgreSQL sessions, bounded I/O and trusted catalog SQL.
//! Authored DDL/checks require closed validation; inspection uses only fixed catalog SQL.
use super::inventory::{Endpoint, Migration};
use super::{Diagnostic, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
use std::time::Duration;
use tokio_postgres::config::SslMode;
use tokio_postgres::{Client, Config, NoTls, Row};

mod scratch;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Apply the per-exchange deadline and expose only SQLSTATE, never server error payloads.
/// The owning adapter must discard its dedicated connection after cancellation/errors.
pub(crate) async fn bounded<T>(
    future: impl Future<Output = std::result::Result<T, tokio_postgres::Error>>,
) -> Result<T> {
    bounded_for(IO_TIMEOUT, future).await
}
/// Authored scripts, index builds and migrator waits get a configured server
/// limit (see `inventory::Limits`); the client deadline sits this far above
/// it, so PostgreSQL reports the timeout before the client gives up.
const SERVER_LIMIT_MARGIN: Duration = Duration::from_secs(5);
/// The settings of the transaction each migration runs in, on the target
/// (`begin_migration`) and in scratch replay (`begin_replay_step`), after
/// `synchronous_commit`.
const MIGRATION_TRANSACTION: &str = "SET LOCAL search_path=pg_catalog; SET LOCAL row_security=off; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces=''; SET LOCAL lock_timeout='1s';";
/// Transaction settings NTNT relies on while inspecting, replaying and
/// executing. Reapplied after each migration's RESET ALL.
const MIGRATION_SETTINGS: &str = "SET LOCAL synchronous_commit=on; SET LOCAL search_path=pg_catalog; SET LOCAL row_security=off; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces=''; SET LOCAL lock_timeout='1s'; SET LOCAL statement_timeout=5000;";
async fn bounded_for<T>(
    limit: Duration,
    future: impl Future<Output = std::result::Result<T, tokio_postgres::Error>>,
) -> Result<T> {
    match tokio::time::timeout(limit, future).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(driver_error(&e)),
        Err(_) => Err(deadline_error()),
    }
}
/// Expose only SQLSTATE, never server error payloads.
fn driver_error(e: &tokio_postgres::Error) -> Diagnostic {
    match e.code().map(|c| c.code()) {
        // A lock wait that hit lock_timeout, or a deadlock victim. Nothing was
        // committed; the caller may retry the whole migration transaction.
        Some(code @ ("55P03" | "40P01")) => Diagnostic::new(
            "DB_LOCK_CONFLICT",
            &format!("A lock was not granted in time (SQLSTATE {code})."),
        ),
        code => Diagnostic::new(
            "DB_POSTGRES",
            &format!(
                "PostgreSQL operation failed (SQLSTATE {}). Server details suppressed.",
                code.unwrap_or("unavailable")
            ),
        ),
    }
}
fn deadline_error() -> Diagnostic {
    Diagnostic::new(
        "DB_TIMEOUT",
        "Database connect/authentication/query deadline exceeded.",
    )
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(crate) struct Postgres {
    client: Client,
    task: tokio::task::JoinHandle<()>,
    /// sslmode, trust source and the TLS version this client negotiated.
    transport: Value,
    /// Set once `prepare_scratch` validated this session as scratch.
    scratch: std::sync::OnceLock<Box<ScratchScope>>,
    /// What a scratch session's replay set up (see `scratch`), boxed so
    /// that `Adapter`'s variants stay alike in size.
    replay: Box<scratch::Replay>,
}
/// What a prepared scratch database held outside system schemas: the
/// managed schema, `public` and NTNT's replay marker. Objects that appear
/// outside the managed scope later are the replayed history's
/// (`outside_objects`).
struct ScratchScope {
    /// The schemas scratch was prepared with; the history did not create them.
    schemas: Vec<String>,
    /// The committed replay marker, as each replayed migration must leave it.
    marker: scratch::Marker,
    /// Shared state the scratch role could change (`scratch::SHARED_STATE`),
    /// as each replayed migration must leave it.
    shared: String,
    /// A replay that never finished, whose leftovers prepare cleaned up.
    recovered: Option<Value>,
}
/// The managed scope: the managed schema first, then the schemas the
/// history creates.
fn scope(schema: &str, extra: &[String]) -> Vec<String> {
    std::iter::once(schema.to_owned())
        .chain(extra.iter().cloned())
        .collect()
}
/// Objects a diagnostic lists at most; its message gives the full count.
const LISTED_OBJECTS: usize = 100;
/// `DB_OUTSIDE_SCOPE` for `migration`, naming `objects`: what it would do
/// in `_ntnt` or a PostgreSQL system schema.
fn protected_schema(migration: &Migration, message: &str, mut objects: Vec<String>) -> Diagnostic {
    let total = objects.len();
    objects.truncate(LISTED_OBJECTS);
    let count = if total > LISTED_OBJECTS {
        format!(" The first {LISTED_OBJECTS} of {total} are listed.")
    } else {
        String::new()
    };
    let mut d = Diagnostic::new(
        "DB_OUTSIDE_SCOPE",
        &format!("{message} The _ntnt schema holds NTNT's migration ledger, _ntnt_replay marks a replay in scratch, and the pg_* and information_schema schemas belong to PostgreSQL, so migrations may not act on them.{count}"),
    )
    .with_objects(&objects);
    d.source = Some(migration.name.clone());
    d
}
impl Drop for Postgres {
    fn drop(&mut self) {
        self.task.abort();
    } // Never return a possibly open session to a pool.
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct Identity {
    pub cluster: String,
    pub database_oid: u32,
    pub database: String,
    pub role: String,
    pub version: i32,
    pub settings: Value,
    pub authority: Value,
    /// sslmode, trust source, the TLS version this client negotiated, and whether
    /// PostgreSQL reports its own session leg as SSL (`server_ssl`). Part of the
    /// plan binding: a plan made over TLS cannot run over a weaker transport.
    pub transport: Value,
}
use super::model::{Catalog, History, HistoryEntry};

/// Small relational boundary: each adapter owns its native catalog and operation
/// capabilities. A driver alone does not confer migration support.
pub(crate) trait RelationalAdapter {
    /// Inspect `schema` and the `extra` schemas of the managed scope.
    /// `outside` lists the objects the history creates in other schemas, by
    /// catalog key; only they are inspected there (see `catalog_sql`).
    async fn catalog(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
    ) -> Result<Catalog>;
    fn capabilities(version: i32) -> Value;
}
/// What reading the relations a check can reach could call through a
/// conversion (see `Postgres::check_purity`): one row per finding, the
/// relation's catalog key and why, for the first 20. `walk` follows each
/// column's type, starting at its top level (`runs` false: comparing with a
/// literal converts it to a domain's base type), and each type a view's
/// rules convert to (`runs` true): the types they depend on, and the
/// `RETURNING` types of SQL/JSON expressions, which no dependency records. Below an array's elements, a range's
/// bounds or a composite's attributes, a type's input runs (`runs` true), and
/// then a domain's constraints run, and so do those of the types they
/// convert to. pg_depend never records a builtin (pinned) function, so what
/// a domain's constraint calls is read from its stored expression; an
/// operator is followed to its function, and to its negator's and
/// commutator's, which the planner may use instead. A view that calls a
/// builtin converting values to a type given at run time
/// (`pg_input_is_valid`, `json_populate_record`, a type's input function)
/// counts while some domain's constraints call such code, since no
/// dependency says which type it reaches. A top-level composite is counted
/// too, though PostgreSQL 16-18 reject a literal compared with one (as an
/// anonymous record) before any input runs. "The history wrote" means
/// outside `pg_catalog` and not installed by an extension in the scope.
const CHECK_CONVERSION_CALLS: &str = r#"WITH RECURSIVE reach(rel) AS (
  SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1
  UNION
  SELECT d.refobjid FROM reach JOIN pg_catalog.pg_class v ON v.oid=reach.rel AND v.relkind='v'
  JOIN pg_catalog.pg_rewrite r ON r.ev_class=v.oid
  JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_rewrite'::pg_catalog.regclass AND d.objid=r.oid AND d.refclassid='pg_catalog.pg_class'::pg_catalog.regclass),
walk(rel, col, top, typ, runs) AS (
  (SELECT a.attrelid, a.attname::text, a.atttypid, a.atttypid, false FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
    WHERE c.oid IN (SELECT rel FROM reach) AND c.relkind IN ('r','p','v') AND a.attnum>0 AND NOT a.attisdropped
   UNION ALL
   SELECT r.ev_class, NULL, d.refobjid, d.refobjid, true FROM pg_catalog.pg_rewrite r
    JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_rewrite'::pg_catalog.regclass AND d.objid=r.oid AND d.refclassid='pg_catalog.pg_type'::pg_catalog.regclass
    WHERE r.ev_class IN (SELECT rel FROM reach)
   UNION ALL
   SELECT r.ev_class, NULL, m[1]::oid, m[1]::oid, true FROM pg_catalog.pg_rewrite r
    CROSS JOIN LATERAL pg_catalog.regexp_matches(r.ev_action::text, ':typid (\d+)', 'g') m
    WHERE r.ev_class IN (SELECT rel FROM reach))
  UNION
  SELECT w.rel, w.col, w.top, x.typ, w.runs OR x.below FROM walk w JOIN pg_catalog.pg_type t ON t.oid=w.typ
  CROSS JOIN LATERAL (
    SELECT t.typbasetype, false WHERE t.typtype='d'
    UNION ALL SELECT t.typelem, true WHERE t.typelem<>0
    UNION ALL SELECT a.atttypid, true FROM pg_catalog.pg_attribute a WHERE t.typrelid<>0 AND a.attrelid=t.typrelid AND a.attnum>0 AND NOT a.attisdropped
    UNION ALL SELECT g.rngsubtype, true FROM pg_catalog.pg_range g WHERE g.rngtypid=t.oid
    UNION ALL SELECT g.rngtypid, true FROM pg_catalog.pg_range g WHERE g.rngmultitypid=t.oid
    UNION ALL SELECT d.refobjid, true FROM pg_catalog.pg_constraint k
     JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_constraint'::pg_catalog.regclass AND d.objid=k.oid AND d.refclassid='pg_catalog.pg_type'::pg_catalog.regclass
     WHERE w.runs AND k.contypid=t.oid) x(typ, below)),
-- Operators a view's rules or a domain's constraints use, with the
-- negators and commutators the planner may put in their place.
used(kind, owner, root, op) AS (
  (SELECT 'v', r.ev_class, d.refobjid, d.refobjid FROM pg_catalog.pg_rewrite r
    JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_rewrite'::pg_catalog.regclass AND d.objid=r.oid AND d.refclassid='pg_catalog.pg_operator'::pg_catalog.regclass
    WHERE r.ev_class IN (SELECT rel FROM reach)
   UNION ALL
   SELECT 'd', k.contypid, d.refobjid, d.refobjid FROM pg_catalog.pg_constraint k
    JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_constraint'::pg_catalog.regclass AND d.objid=k.oid AND d.refclassid='pg_catalog.pg_operator'::pg_catalog.regclass
    WHERE k.contypid<>0)
  UNION
  SELECT u.kind, u.owner, u.root, x.op FROM used u JOIN pg_catalog.pg_operator o ON o.oid=u.op
  CROSS JOIN LATERAL (VALUES (o.oprnegate), (o.oprcom)) x(op) WHERE x.op<>0),
-- What each domain's constraints call. pg_depend records no builtin
-- function, so calls are read from the stored expression.
calls(dom, fn) AS (
  SELECT k.contypid, m[1]::oid FROM pg_catalog.pg_constraint k CROSS JOIN LATERAL pg_catalog.regexp_matches(k.conbin::text, ':(?:funcid|opfuncid) (\d+)', 'g') m WHERE k.contypid<>0
  UNION ALL
  SELECT u.owner, o.oprcode FROM used u JOIN pg_catalog.pg_operator o ON o.oid=u.op WHERE u.kind='d'),
-- A routine the history wrote (outside pg_catalog, not an extension's in
-- the scope) or a volatile function.
unsafe(fn, why) AS (
  SELECT p.oid, CASE WHEN pn.nspname<>'pg_catalog' AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend e JOIN pg_catalog.pg_extension x ON x.oid=e.refobjid
    JOIN pg_catalog.pg_namespace xn ON xn.oid=x.extnamespace WHERE e.classid='pg_catalog.pg_proc'::pg_catalog.regclass AND e.objid=p.oid
    AND e.refclassid='pg_catalog.pg_extension'::pg_catalog.regclass AND e.deptype='e' AND xn.nspname=ANY($2))
    THEN ' (a routine the history wrote)' WHEN p.provolatile='v' THEN ' (a volatile function)' END
  FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace
  WHERE p.oid IN (SELECT fn FROM calls UNION SELECT o.oprcode FROM used u JOIN pg_catalog.pg_operator o ON o.oid=u.op)),
-- Builtins that convert values to a type given at run time (by name, as an
-- OID or as a record argument), which no dependency records: they can reach
-- any domain, so they count while some domain's constraints call one of
-- the functions above.
dynamic(fn) AS (
  SELECT p.oid FROM pg_catalog.pg_proc p WHERE p.pronamespace='pg_catalog'::pg_catalog.regnamespace
  AND p.proname=ANY('{pg_input_is_valid,pg_input_error_info,json_populate_record,json_populate_recordset,jsonb_populate_record,jsonb_populate_recordset,jsonb_populate_record_valid,record_in,record_recv,domain_in,domain_recv,array_in,array_recv,range_in,range_recv,multirange_in,multirange_recv}'::text[])
  AND EXISTS(SELECT 1 FROM calls c JOIN unsafe u ON u.fn=c.fn WHERE u.why IS NOT NULL)),
why(fn, txt) AS (
  SELECT fn, fn::pg_catalog.regprocedure::text||why FROM unsafe WHERE why IS NOT NULL
  UNION ALL
  SELECT fn, fn::pg_catalog.regprocedure::text||' (which converts values to a type given at run time, while domain constraints here call such functions)' FROM dynamic),
found(rel, what) AS (
  SELECT w.rel, CASE WHEN w.col IS NULL THEN 'converts values to '||pg_catalog.format_type(w.top,NULL)||', which runs'
    ELSE 'has column '||pg_catalog.quote_ident(w.col)||' of type '||pg_catalog.format_type(w.top,NULL)||', and converting a literal to that type runs' END
    ||' the constraints of domain '||pg_catalog.format_type(w.typ,NULL)||', which call '||y.txt
  FROM walk w JOIN pg_catalog.pg_type t ON t.oid=w.typ AND t.typtype='d' JOIN calls c ON c.dom=t.oid JOIN why y ON y.fn=c.fn WHERE w.runs
  UNION ALL
  SELECT u.owner, 'uses operator '||u.root::pg_catalog.regoperator::text||CASE WHEN u.op<>u.root THEN ', which the planner may replace with '||u.op::pg_catalog.regoperator::text ELSE '' END
    ||', whose function is '||y.txt FROM used u JOIN pg_catalog.pg_operator o ON o.oid=u.op JOIN why y ON y.fn=o.oprcode WHERE u.kind='v'
  UNION ALL
  SELECT r.ev_class, 'calls '||y.txt FROM pg_catalog.pg_rewrite r CROSS JOIN LATERAL pg_catalog.regexp_matches(r.ev_action::text, ':(?:funcid|opfuncid) (\d+)', 'g') m
   JOIN dynamic d ON d.fn=m[1]::oid JOIN why y ON y.fn=d.fn WHERE r.ev_class IN (SELECT rel FROM reach))
SELECT DISTINCT CASE WHEN rn.nspname=$1 THEN '' ELSE pg_catalog.quote_ident(rn.nspname)||'.' END||'relation:'||rc.relname,
  CASE WHEN rc.relkind='v' THEN 'view ' ELSE 'table ' END||pg_catalog.quote_ident(rn.nspname)||'.'||pg_catalog.quote_ident(rc.relname)||' '||f.what
FROM found f JOIN pg_catalog.pg_class rc ON rc.oid=f.rel JOIN pg_catalog.pg_namespace rn ON rn.oid=rc.relnamespace
ORDER BY 1, 2 LIMIT 20"#;

impl Postgres {
    /// Open a dedicated session after checking explicit host, role, database and transport.
    /// The initial connection is read-only by default. Every TLS sslmode requires TLS from
    /// the server (no plaintext fallback); `disable` is plaintext for a trusted local transport.
    pub async fn connect(
        secret: &crate::interpreter::SecretValue,
        endpoint: &Endpoint,
    ) -> Result<Self> {
        let (connection, transport) = super::postgres_tls::split(secret.expose())?;
        let mut config: Config = connection.as_str().parse().map_err(|_| {
            Diagnostic::new(
                "DB_CONNECTION_CONFIG",
                "Invalid connection secret; details suppressed.",
            )
        })?;
        if config.get_dbname() != Some(&endpoint.database)
            || config.get_user() != Some(&endpoint.role)
            || config.get_hosts().len() != 1
            || config.get_options().is_some()
        {
            return Err(Diagnostic::new("DB_CONNECTION_CONFIG","Use one explicit host, the declared database/role, an explicit sslmode, and no startup options."));
        }
        let encrypted = transport.mode() != super::postgres_tls::Mode::Disable;
        if encrypted
            && !matches!(
                config.get_hosts().first(),
                Some(tokio_postgres::config::Host::Tcp(_))
            )
        {
            return Err(Diagnostic::new(
                "DB_CONNECTION_CONFIG",
                "TLS needs a TCP host name or address. A Unix-socket directory is a local transport: use sslmode=disable for it.",
            ));
        }
        if config.get_channel_binding() == tokio_postgres::config::ChannelBinding::Require {
            return Err(Diagnostic::new(
                "DB_CONNECTION_CONFIG",
                "channel_binding=require is not supported; sslmode=verify-full authenticates the server.",
            ));
        }
        // The TLS keywords were removed above, so set the mode explicitly:
        // tokio-postgres would otherwise default to `prefer`.
        config.ssl_mode(if encrypted {
            SslMode::Require
        } else {
            SslMode::Disable
        });
        config.connect_timeout(IO_TIMEOUT);
        config.application_name("ntnt-db-inspect-v1");
        config.options("-c default_transaction_read_only=on -c search_path=pg_catalog -c statement_timeout=5000 -c lock_timeout=2000 -c idle_in_transaction_session_timeout=10000 -c standard_conforming_strings=on -c row_security=off");
        let (connector, mut transport) = transport.connector().await?;
        let (client, task) = match connector {
            None => {
                let (client, connection) = bounded(config.connect(NoTls)).await?;
                let task = tokio::spawn(async move {
                    let _ = connection.await;
                });
                (client, task)
            }
            Some(tls) => {
                let negotiated = tls.clone();
                let (client, connection) =
                    match tokio::time::timeout(IO_TIMEOUT, config.connect(tls)).await {
                        Ok(Ok(pair)) => pair,
                        Ok(Err(e)) => {
                            return Err(super::postgres_tls::classify(&e)
                                .unwrap_or_else(|| driver_error(&e)))
                        }
                        Err(_) => return Err(deadline_error()),
                    };
                let task = tokio::spawn(async move {
                    let _ = connection.await;
                });
                // What this client negotiated is the transport it controls; a
                // TLS-terminating proxy does not change it.
                transport["tls_version"] = json!(negotiated.negotiated_version());
                (client, task)
            }
        };
        Ok(Self {
            client,
            task,
            transport,
            scratch: std::sync::OnceLock::new(),
            replay: Box::default(),
        })
    }
    pub async fn batch(&self, sql: &str) -> Result<()> {
        bounded(self.client.batch_execute(sql)).await
    }
    pub async fn rows(
        &self,
        sql: &str,
        args: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Vec<Row>> {
        let rows = bounded(self.client.query(sql, args)).await?;
        if rows.len() > 10000 {
            return Err(Diagnostic::new(
                "DB_CATALOG_LIMIT",
                "Inspection exceeded 10,000 rows per object class.",
            ));
        }
        Ok(rows)
    }
    pub async fn begin_readonly(&self) -> Result<()> {
        self.batch("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL search_path=pg_catalog; SET LOCAL row_security=off;").await
    }
    pub async fn rollback(&self) -> Result<()> {
        self.batch("ROLLBACK").await
    }
    /// Observe physical cluster/database identity, session settings and role authority.
    /// Reject endpoint mismatches or unsupported engine versions before trusting a plan.
    pub async fn identity(&self, endpoint: &Endpoint) -> Result<Identity> {
        let r = bounded(self.client.query_one(
            r#"
SELECT (pg_catalog.pg_control_system()).system_identifier::text, d.oid, d.datname::text,
       current_user::text, pg_catalog.current_setting('server_version_num')::integer,
       pg_catalog.jsonb_build_object('encoding',pg_catalog.pg_encoding_to_char(d.encoding),
          'collate',d.datcollate,'ctype',d.datctype,'locale_provider',d.datlocprovider,
          'collation_version',d.datcollversion,'locale',pg_catalog.to_jsonb(d)->>'datlocale',
          'icu_locale',pg_catalog.to_jsonb(d)->>'daticulocale',
          'standard_conforming_strings',pg_catalog.current_setting('standard_conforming_strings'),
          'timezone',pg_catalog.current_setting('TimeZone'),
          'date_style',pg_catalog.current_setting('DateStyle'),
          'interval_style',pg_catalog.current_setting('IntervalStyle'),
          'client_encoding',pg_catalog.current_setting('client_encoding'))
FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()
"#,
            &[],
        ))
        .await?;
        let authority=bounded(self.client.query_one(r#"
WITH RECURSIVE roles(oid) AS (
 SELECT oid FROM pg_catalog.pg_roles WHERE rolname=current_user
 UNION SELECT m.roleid FROM pg_catalog.pg_auth_members m JOIN roles r ON m.member=r.oid)
SELECT jsonb_build_object('database_owner',(SELECT pg_get_userbyid(datdba) FROM pg_catalog.pg_database WHERE datname=current_database()),
 'roles',(SELECT jsonb_agg(jsonb_build_object('role',r.rolname,'superuser',r.rolsuper,'createdb',r.rolcreatedb,'createrole',r.rolcreaterole,'inherit',r.rolinherit,'bypassrls',r.rolbypassrls,'replication',r.rolreplication) ORDER BY r.rolname) FROM pg_catalog.pg_roles r JOIN roles ON r.oid=roles.oid),
 'memberships',(SELECT jsonb_agg(jsonb_build_object('member',pg_get_userbyid(m.member),'role',pg_get_userbyid(m.roleid),'admin',m.admin_option,'inherit',pg_catalog.to_jsonb(m)->'inherit_option','set',pg_catalog.to_jsonb(m)->'set_option') ORDER BY pg_get_userbyid(m.member),pg_get_userbyid(m.roleid)) FROM pg_catalog.pg_auth_members m JOIN roles r ON m.member=r.oid))
"#,&[])).await?.get::<_,Value>(0);
        // The server's own view of this session. Directly connected it matches
        // the client; behind a TLS-terminating proxy or pooler it describes the
        // proxy's leg and can be false while the client connection is verified TLS.
        let server_ssl: bool = bounded(self.client.query_one(
            "SELECT s.ssl FROM pg_catalog.pg_stat_ssl s WHERE s.pid = pg_catalog.pg_backend_pid()",
            &[],
        ))
        .await?
        .get(0);
        let mut transport = self.transport.clone();
        if transport.get("tls_version").is_none() {
            transport["tls_version"] = Value::Null;
        }
        transport["server_ssl"] = json!(server_ssl);
        let identity = Identity {
            cluster: r.get(0),
            database_oid: r.get(1),
            database: r.get(2),
            role: r.get(3),
            version: r.get(4),
            settings: r.get(5),
            authority,
            transport,
        };
        if identity.database != endpoint.database || identity.role != endpoint.role {
            return Err(Diagnostic::new(
                "DB_WRONG_TARGET",
                "Connected database/role does not match the declared endpoint.",
            ));
        }
        if ![16, 17, 18].contains(&(identity.version / 10000)) {
            return Err(Diagnostic::new(
                "DB_ENGINE_VERSION",
                "Catalog adapter supports PostgreSQL 16–18 only.",
            ));
        }
        Ok(identity)
    }
    /// Read history only after validating the reserved ledger's physical shape and policies.
    /// Local unknown objects fail closed; unrelated global coverage gaps are reported
    /// separately and must not masquerade as corrupt migration history.
    pub async fn history(&self) -> Result<History> {
        let ns = self
            .rows(
                "SELECT oid FROM pg_catalog.pg_namespace WHERE nspname='_ntnt'",
                &[],
            )
            .await?;
        if ns.is_empty() {
            return Ok(History {
                present: false,
                entries: vec![],
                scope: None,
            });
        }
        let rels = self.rows("SELECT c.oid,c.relname::text,c.relkind::text,c.relrowsecurity,c.relforcerowsecurity FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='_ntnt' AND c.relkind NOT IN ('i','S')",&[]).await?;
        let bad = || {
            Diagnostic::new("DB_LEDGER_UNKNOWN","Unrecognized, unreadable, or policy-filtered _ntnt ledger; explicit adoption/reconciliation is required.")
        };
        if rels.len() != 1
            || !["schema_migrations_v1", "schema_migrations_v2"]
                .contains(&rels[0].get::<_, String>(1).as_str())
            || rels[0].get::<_, String>(2) != "r"
            || rels[0].get::<_, bool>(3)
            || rels[0].get::<_, bool>(4)
        {
            return Err(bad());
        }
        let table: String = rels[0].get(1);
        let v2 = table == "schema_migrations_v2";
        self.batch(&format!(
            "LOCK TABLE ONLY _ntnt.{table} IN ACCESS SHARE MODE"
        ))
        .await
        .map_err(|_| bad())?;
        let attrs=self.rows("SELECT a.attname::text,a.atttypid,a.attnotnull FROM pg_catalog.pg_attribute a WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",&[&rels[0].get::<_,u32>(0)]).await?;
        let mut expected = vec![
            ("position", 20u32),
            ("id", 25),
            ("name", 25),
            ("checksum", 25),
            ("state", 25),
        ];
        if v2 {
            expected.extend([("logical_database", 25), ("managed_schema", 25)]);
        }
        if attrs.len() != expected.len()
            || attrs.iter().zip(expected).any(|(a, (n, t))| {
                a.get::<_, String>(0) != n || a.get::<_, u32>(1) != t || !a.get::<_, bool>(2)
            })
        {
            return Err(bad());
        }
        let checks=self.rows("SELECT c.oid=$1 AND NOT c.relrowsecurity AND NOT c.relforcerowsecurity AND NOT c.relispartition AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_inherits WHERE inhrelid=c.oid) AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_rewrite WHERE ev_class=c.oid) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON c.relnamespace=n.oid WHERE n.nspname='_ntnt' AND c.oid=$1",&[&rels[0].get::<_,u32>(0)]).await?;
        if checks.len() != 1 || !checks[0].get::<_, bool>(0) {
            return Err(bad());
        }
        let ledger_scope = self
            .catalog("_ntnt", &[], &[], "")
            .await
            .map_err(|_| bad())?;
        if ledger_scope.uncovered.iter().any(|key| {
            ledger_scope
                .objects
                .get(key)
                .is_none_or(|object| object["external"] != true)
        }) {
            return Err(bad());
        }
        // Covered is not the same as expected: the reserved namespace holds only
        // the ledger heap and its own columns, constraints and indexes.
        if ledger_scope.objects.iter().any(|(key, object)| {
            object["external"] != true
                && !["schema:", "relation:", "column:", "constraint:", "index:"]
                    .iter()
                    .any(|prefix| key.starts_with(prefix))
        }) {
            return Err(bad());
        }
        // The ledger is read only after its physical table and every dependent
        // index are known built-in objects. A SELECT on a view/RLS policy/custom
        // access method is not a pure history read.
        let safe = self.rows(r#"SELECT am.amname='heap' AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid=i.indexrelid
            JOIN pg_catalog.pg_am ia ON ia.oid=ic.relam WHERE i.indrelid=c.oid AND
            (ia.amname<>'btree' OR i.indexprs IS NOT NULL OR i.indpred IS NOT NULL OR EXISTS (
              SELECT 1 FROM unnest(i.indclass) x JOIN pg_catalog.pg_opclass o ON o.oid=x
              JOIN pg_catalog.pg_namespace n ON n.oid=o.opcnamespace WHERE n.nspname<>'pg_catalog')))
            FROM pg_catalog.pg_class c JOIN pg_catalog.pg_am am ON am.oid=c.relam WHERE c.oid=$1"#,
            &[&rels[0].get::<_,u32>(0)]).await.map_err(|_| bad())?;
        if safe.len() != 1 || !safe[0].get::<_, bool>(0) {
            return Err(bad());
        }
        let rows=self.rows(&format!("SELECT position,pg_catalog.left(id,21),pg_catalog.left(name,256),pg_catalog.left(checksum,65),pg_catalog.left(state,32),octet_length(id)<=20 AND octet_length(name)<=255 AND octet_length(checksum)=64 AND octet_length(state)<=31 FROM ONLY _ntnt.{table} ORDER BY position LIMIT 10001"),&[]).await.map_err(|_| bad())?;
        let scope = if v2 {
            let scopes = self.rows("SELECT DISTINCT pg_catalog.left(logical_database,64),pg_catalog.left(managed_schema,64) FROM ONLY _ntnt.schema_migrations_v2 LIMIT 2", &[]).await?;
            if scopes.len() != 1 {
                return Err(bad());
            }
            Some(
                json!({"database":scopes[0].get::<_,String>(0),"schema":scopes[0].get::<_,String>(1)}),
            )
        } else {
            None
        };
        if rows.iter().any(|r| !r.get::<_, bool>(5)) {
            return Err(bad());
        }
        Ok(History {
            present: true,
            scope,
            entries: rows
                .into_iter()
                .map(|r| HistoryEntry {
                    position: r.get(0),
                    id: r.get(1),
                    name: r.get(2),
                    checksum: r.get(3),
                    state: r.get(4),
                })
                .collect(),
        })
    }
    pub async fn validate_execution_scope(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
    ) -> Result<()> {
        // A cross-scope dependent is not an owned object. Even non-cascading changes
        // are conservatively rejected rather than promising a transitive effect bound.
        // A table is tracked when it is in a schema of the managed scope or is
        // one the history creates elsewhere (`outside`, by catalog key). A
        // foreign key from an untracked table into a tracked one blocks
        // execution, whichever kind the tracked one is: a DELETE there could
        // cascade into rows nothing inspects. One between two tracked tables
        // does not.
        let incoming = self.rows("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.confrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_class f ON f.oid=k.conrelid JOIN pg_catalog.pg_namespace fn ON fn.oid=f.relnamespace WHERE (n.nspname=ANY($1) OR quote_ident(n.nspname)||'.relation:'||c.relname=ANY($2)) AND fn.nspname<>ALL($1) AND quote_ident(fn.nspname)||'.relation:'||f.relname<>ALL($2))", &[&scope(schema, extra), &outside]).await?;
        if incoming[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_EXTERNAL_DEPENDENCY",
                "Incoming foreign keys from outside the managed scope block execution.",
            ));
        }
        Ok(())
    }

    /// Acquire database-local ownership of the single reserved ledger namespace.
    /// This session lock survives per-migration commits. Drop discards the socket;
    /// it cannot leak into a pool. PostgreSQL lock_timeout bounds competing owners.
    /// A second migrator waits up to `wait` for the first, then gives up with a
    /// lock conflict: the statement and client deadlines sit above that wait.
    pub async fn own_migrations(&self, wait: Duration) -> Result<()> {
        self.batch(&format!(
            "SET lock_timeout = '{}s'; SET statement_timeout = '{}s'",
            wait.as_secs(),
            wait.as_secs() + 1
        ))
        .await?;
        let owned = bounded_for(
            wait + SERVER_LIMIT_MARGIN,
            self.client
                .query("SELECT pg_catalog.pg_advisory_lock(1853124212, 245)", &[]),
        )
        .await;
        self.batch("SET lock_timeout = '2s'; SET statement_timeout = 5000")
            .await?;
        owned.map(|_| ())
    }

    /// The advisory lock already serializes migrators. Each statement takes only
    /// the locks PostgreSQL requires for it, and a lock that is not granted
    /// within one second fails the attempt instead of queueing application
    /// queries behind it; the caller retries the whole transaction.
    pub async fn begin_migration(&self, _schema: &str) -> Result<()> {
        self.batch(&format!(
            "BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE; SET LOCAL synchronous_commit=on; {MIGRATION_TRANSACTION}"
        ))
        .await
    }

    /// Tables read by a migration's checks are locked in SHARE mode before the
    /// first check: writers wait, readers do not. READ COMMITTED takes each
    /// check's snapshot after this lock, so checked data cannot change under it.
    /// Without ONLY, a partitioned table's partitions are locked as well, so
    /// writes made directly to a partition also wait.
    pub async fn lock_for_checks(&self, tables: &[(String, String)]) -> Result<()> {
        for (schema, table) in tables {
            self.batch(&format!(
                "LOCK TABLE {}.{} IN SHARE MODE",
                quote(schema),
                quote(table)
            ))
            .await?;
        }
        Ok(())
    }

    /// Validity of an index in the managed schema, or None if absent.
    pub async fn index_validity(&self, schema: &str, name: &str) -> Result<Option<bool>> {
        Ok(self
            .rows("SELECT i.indisvalid FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indexrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2", &[&schema, &name])
            .await?
            .first()
            .map(|r| r.get(0)))
    }
    /// Run statements outside any transaction. The session is read-only by
    /// default; lift that only for this call. Uses replay's storage settings
    /// so the result matches the plan's catalog. Settings are restored on
    /// every path, and a build error is reported in preference to a restore
    /// error. `limit` bounds the statement, waits and table scan included.
    async fn outside_transaction(&self, sql: &str, schema: &str, limit: Duration) -> Result<()> {
        let result = async {
            self.batch(&format!(
                "SET default_transaction_read_only=off; SET search_path=pg_catalog,{}; SET default_tablespace=''; SET temp_tablespaces=''; SET lock_timeout=0; SET statement_timeout='{}s';",
                quote(schema),
                limit.as_secs()
            ))
            .await?;
            bounded_for(
                limit + SERVER_LIMIT_MARGIN,
                self.client.batch_execute(sql),
            )
            .await
        }
        .await;
        let restored = self
            .batch("RESET default_tablespace; RESET temp_tablespaces; SET default_transaction_read_only=on; SET search_path=pg_catalog; SET lock_timeout='2s'; SET statement_timeout=5000;")
            .await;
        result.and(restored)
    }
    /// Build a reviewed CREATE INDEX CONCURRENTLY. It takes only SHARE UPDATE
    /// EXCLUSIVE, which does not block reads or writes, and waits (up to
    /// `limit`) for earlier transactions instead of timing out on them.
    pub async fn build_concurrent_index(
        &self,
        migration: &Migration,
        schema: &str,
        extra: &[String],
        limit: Duration,
    ) -> Result<()> {
        // Index expressions coerce values like any DDL; see execute_captured.
        self.reject_custom_casts(&scope(schema, extra))
            .await
            .map_err(|mut d| {
                d.source = Some(migration.name.clone());
                d
            })?;
        self.outside_transaction(&migration.sql, schema, limit)
            .await
            .map_err(|mut d| {
                d.source = Some(migration.name.clone());
                d
            })
    }
    /// Whether a CREATE INDEX of this name is running in any session now.
    pub async fn index_building(&self, schema: &str, name: &str) -> Result<bool> {
        Ok(self
            .rows("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_progress_create_index p JOIN pg_catalog.pg_class c ON c.oid=p.index_relid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2)", &[&schema, &name])
            .await?[0]
            .get(0))
    }
    /// Move a recorded `running` attempt to `applied`, or delete it. Exactly
    /// one row must change; anything else leaves the transaction to roll back.
    pub async fn resolve_attempt(&self, position: usize, applied: bool) -> Result<()> {
        let position = position as i64;
        let sql = if applied {
            "UPDATE _ntnt.schema_migrations_v2 SET state='applied' WHERE position=$1 AND state='running' RETURNING position"
        } else {
            "DELETE FROM _ntnt.schema_migrations_v2 WHERE position=$1 AND state='running' RETURNING position"
        };
        if self.rows(sql, &[&position]).await?.len() != 1 {
            return Err(Diagnostic::new(
                "DB_RECONCILE_MISMATCH",
                "The running attempt changed while it was being resolved; nothing was changed.",
            ));
        }
        Ok(())
    }
    /// A failed or interrupted concurrent build leaves an invalid index.
    pub async fn drop_invalid_index(
        &self,
        schema: &str,
        name: &str,
        limit: Duration,
    ) -> Result<()> {
        self.outside_transaction(
            &format!(
                "DROP INDEX CONCURRENTLY IF EXISTS {}.{}",
                quote(schema),
                quote(name)
            ),
            schema,
            limit,
        )
        .await
    }
    /// Relation locks this transaction holds on managed tables: what the
    /// migration actually took, observed on the target before commit.
    pub async fn relation_locks(&self, schema: &str) -> Result<Value> {
        let rows = self.rows("SELECT c.relname::text, l.mode FROM pg_catalog.pg_locks l JOIN pg_catalog.pg_class c ON c.oid=l.relation JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE l.pid=pg_catalog.pg_backend_pid() AND l.locktype='relation' AND l.granted AND n.nspname=$1 AND c.relkind IN ('r','p') ORDER BY 1,2", &[&schema]).await?;
        Ok(json!(rows
            .iter()
            .map(|r| json!({"relation":r.get::<_, String>(0),"mode":r.get::<_, String>(1)}))
            .collect::<Vec<_>>()))
    }

    /// A readable legacy ledger is not a writable one. Only our scoped v2 heap,
    /// owned by this role and without defaults/triggers/rules/checks/FKs, may receive
    /// an INSERT. The reserved namespace supports one logical store per database.
    pub async fn writable_history(&self, database: &str, schema: &str) -> Result<History> {
        let history = self.history().await?;
        if !history.present {
            return Ok(history);
        }
        if history.scope != Some(json!({"database":database,"schema":schema})) {
            return Err(Diagnostic::new("DB_LEDGER_SCOPE", "Execution requires the scoped v2 ledger for this logical store/schema. Legacy or differently scoped history is read-only; no adoption is automatic."));
        }
        self.mutation_safe_ledger("schema_migrations_v2").await?;
        Ok(history)
    }
    async fn mutation_safe_ledger(&self, table: &str) -> Result<()> {
        let safe = self.rows(r#"SELECT n.nspowner=r.oid AND c.relowner=r.oid AND c.relpersistence='p'
          AND (SELECT count(*)=1 FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_index i ON i.indexrelid=k.conindid
               WHERE k.conrelid=c.oid AND k.contype='p' AND k.conkey=ARRAY[1]::smallint[]
               AND NOT k.condeferrable AND k.convalidated AND i.indisvalid AND i.indisready AND i.indislive)
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_attrdef WHERE adrelid=c.oid)
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_trigger WHERE tgrelid=c.oid)
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_rewrite WHERE ev_class=c.oid)
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint WHERE conrelid=c.oid AND contype NOT IN ('p','n'))
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_attribute WHERE attrelid=c.oid AND (attisdropped OR attidentity<>'' OR attgenerated<>''))
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.aclexplode(coalesce(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) a WHERE a.grantee<>r.oid)
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.aclexplode(coalesce(c.relacl,pg_catalog.acldefault('r',c.relowner))) a WHERE a.grantee<>r.oid)
          FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
          JOIN pg_catalog.pg_roles r ON r.rolname=current_user
          WHERE n.nspname='_ntnt' AND c.relname=$1"#, &[&table]).await?;
        if safe.len() != 1 || !safe[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_LEDGER_UNSAFE",
                "Ledger is not a supported mutation-safe owned heap; no write was attempted.",
            ));
        }
        Ok(())
    }

    /// Import only the recognized, immutable, owned v1 ledger under native locks.
    /// Preserve every source identity/checksum; imported is not newly executed SQL.
    pub async fn import_legacy_history(&self, database: &str, schema: &str) -> Result<()> {
        let history = self.history().await?;
        if !history.present
            || history.scope.is_some()
            || history.entries.is_empty()
            || history.entries.iter().any(|e| e.state != "applied")
        {
            return Err(Diagnostic::new(
                "DB_ADOPTION_INPUT",
                "Only a recognized nonempty legacy v1 ledger can be imported.",
            ));
        }
        self.mutation_safe_ledger("schema_migrations_v1").await?;
        self.batch("ALTER TABLE _ntnt.schema_migrations_v1 RENAME TO schema_migrations_v2; ALTER TABLE _ntnt.schema_migrations_v2 ADD COLUMN logical_database text, ADD COLUMN managed_schema text;").await?;
        self.rows("UPDATE _ntnt.schema_migrations_v2 SET logical_database=$1,managed_schema=$2,state='imported'",&[&database,&schema]).await?;
        self.batch("ALTER TABLE _ntnt.schema_migrations_v2 ALTER COLUMN logical_database SET NOT NULL, ALTER COLUMN managed_schema SET NOT NULL").await?;
        Ok(())
    }

    /// Bootstrap only inside the first authorized migration transaction. Empty or
    /// unrecognized existing namespaces are never reset or automatically adopted.
    pub async fn record_migration(
        &self,
        migration: &Migration,
        position: usize,
        database: &str,
        schema: &str,
        present: bool,
    ) -> Result<()> {
        self.record_entry(
            &HistoryEntry {
                position: position as i64,
                id: migration.id.to_string(),
                name: migration.name.clone(),
                checksum: migration.checksum.clone(),
                state: "applied".into(),
            },
            database,
            schema,
            present,
        )
        .await
    }
    /// Insert a validated record inside the caller-owned migration transaction.
    /// Only an absent reserved ledger may be bootstrapped; eligibility is checked upstream.
    pub async fn record_entry(
        &self,
        entry: &HistoryEntry,
        database: &str,
        schema: &str,
        present: bool,
    ) -> Result<()> {
        if !present {
            self.batch("CREATE SCHEMA _ntnt; CREATE TABLE _ntnt.schema_migrations_v2 (position bigint NOT NULL PRIMARY KEY,id text NOT NULL,name text NOT NULL,checksum text NOT NULL,state text NOT NULL,logical_database text NOT NULL,managed_schema text NOT NULL)").await?;
        }
        self.rows("INSERT INTO _ntnt.schema_migrations_v2(position,id,name,checksum,state,logical_database,managed_schema) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&entry.position,&entry.id,&entry.name,&entry.checksum,&entry.state,&database,&schema]).await?;
        Ok(())
    }

    /// Validate isolation, compatible settings and restricted ownership, then
    /// commit NTNT's replay marker and the managed schema (see `scratch`).
    /// From the replay lock on, which waits up to `wait` for another replay
    /// of this scratch database, the caller must end with `cleanup_scratch`
    /// on every success/error exit, or discard the session. A marker that an
    /// unfinished replay left is cleaned up first when nothing in scratch
    /// has another owner (`scratch_recovered` says so); anything else
    /// nonempty is refused, and never reset or adopted.
    pub async fn prepare_scratch(
        &self,
        target: &Identity,
        endpoint: &Endpoint,
        schema: &str,
        wait: Duration,
    ) -> Result<Identity> {
        self.begin_readonly().await?;
        let identity = self.identity(endpoint).await?;
        if target.cluster == identity.cluster && target.database_oid == identity.database_oid {
            return Err(Diagnostic::new("DB_SCRATCH_COLLISION","Scratch resolves to the deployment database, regardless of connection-string spelling."));
        }
        if target.role == identity.role {
            return Err(Diagnostic::new(
                "DB_SCRATCH_CREDENTIALS",
                "Scratch must use a separate role, not deployment credentials.",
            ));
        }
        if target.version != identity.version || target.settings != identity.settings {
            return Err(Diagnostic::new("DB_SCRATCH_MISMATCH","Scratch engine version, encoding, locale and session settings must exactly match the target."));
        }
        self.rollback().await?;
        // Scripts must see row-level security as the target role does. Scratch
        // may bypass RLS only when the target login role bypasses it (directly
        // or as a superuser); it never gains other elevated attributes.
        let target_bypasses_rls = target.authority["roles"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|r| {
                r["role"] == target.role.as_str()
                    && (r["bypassrls"] == true || r["superuser"] == true)
            });
        let _ = self.replay.bypass_rls.set(target_bypasses_rls);
        // Two replays of one scratch database never interleave: the second
        // waits here, before it reads what scratch holds.
        self.lock_scratch(wait).await?;
        self.batch("BEGIN ISOLATION LEVEL REPEATABLE READ READ WRITE; SET LOCAL search_path=pg_catalog; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces='';").await?;
        // Cleaned up in this transaction, so a refusal below keeps the
        // leftovers, and their marker, as they were.
        let recovered = self.recover_leftovers().await?;
        self.check_empty(target_bypasses_rls).await?;
        let (public, shared) = self.scratch_baseline().await?;
        self.check_cleanable(public.oid).await?;
        self.create_marker(&public).await?;
        // `public` already exists (and was just verified empty); any other
        // managed schema is created and committed with the marker.
        if schema != "public" {
            self.batch(&format!("CREATE SCHEMA \"{schema}\";")).await?;
        }
        // Scratch holds nothing else, so whatever appears outside the managed
        // scope from here on is the replayed history's (`outside_objects`).
        // It has no _ntnt either: a replayed migration that creates one is
        // refused (see execute_captured).
        let schemas = self
            .rows("SELECT n.nspname::text FROM pg_catalog.pg_namespace n WHERE n.nspname<>'information_schema' AND NOT starts_with(n.nspname,'pg_') ORDER BY 1", &[])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        let marker = self.own_marker().await?;
        self.batch("COMMIT").await?;
        self.scratch
            .set(Box::new(ScratchScope {
                schemas,
                marker,
                shared,
                recovered,
            }))
            .map_err(|_| {
                Diagnostic::new("DB_SCRATCH_NOT_EMPTY", "Scratch was already prepared.")
            })?;
        Ok(identity)
    }
    /// The emptiness check of `prepare_scratch`, which cleanup runs again:
    /// an unprivileged, non-member owner role (`bypass_rls` as the target's
    /// role) and nothing but the empty stock `public` and system objects.
    async fn check_empty(&self, bypass_rls: bool) -> Result<()> {
        let r=bounded(self.client.query_one(r#"
SELECT NOT (r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR (r.rolbypassrls AND NOT $1))
 AND d.datdba=r.oid AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members WHERE member=r.oid)
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_event_trigger)
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname<>'plpgsql')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_default_acl)
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname NOT IN ('public','information_schema') AND nspname NOT LIKE 'pg_%')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='public')
FROM pg_catalog.pg_roles r,pg_catalog.pg_database d WHERE r.rolname=current_user AND d.datname=pg_catalog.current_database()
"#,&[&bypass_rls])).await?;
        if !r.get::<_, bool>(0) {
            return Err(scratch::not_empty());
        }
        let public = self.catalog("public", &[], &[], "").await?;
        if public.objects.len() != 1
            || !public.objects.contains_key("schema:public")
            || !public.uncovered.is_empty()
        {
            return Err(Diagnostic::new("DB_SCRATCH_NOT_EMPTY","Scratch public namespace contains unsupported objects or cannot be fully inspected."));
        }
        Ok(())
    }
    /// The replay whose leftovers `prepare_scratch` cleaned up, if any.
    pub fn scratch_recovered(&self) -> Option<Value> {
        self.scratch.get().and_then(|s| s.recovered.clone())
    }
    /// In a prepared scratch database: what the replayed history created
    /// outside the managed scope (`schema` and the `extra` schemas the history
    /// creates), by catalog key (`OUTSIDE_OBJECTS`). Pass it to `catalog` so
    /// they are inspected. Empty in any other session.
    pub async fn outside_objects(&self, schema: &str, extra: &[String]) -> Result<Vec<String>> {
        let Some(scratch) = self.scratch.get() else {
            return Ok(vec![]);
        };
        // Routine keys spell argument types as the catalog's do.
        self.batch("SET LOCAL search_path=pg_catalog").await?;
        Ok(self
            .rows(
                super::catalog_sql::OUTSIDE_OBJECTS,
                &[&scope(schema, extra), &scratch.schemas],
            )
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect())
    }
    /// Reject custom implicit/assignment casts database-wide before DDL:
    /// ordinary coercion could run them without appearing in the reviewed
    /// script. The casts of an extension installed in the managed `scope`
    /// (citext's, say) are the extension's own code, which its version
    /// covers like its routines and operators, so they are allowed.
    async fn reject_custom_casts(&self, scope: &[String]) -> Result<()> {
        let casts: Vec<String> = self
            .rows("SELECT 'cast:'||pg_catalog.format_type(c.castsource,NULL)||' AS '||pg_catalog.format_type(c.casttarget,NULL) FROM pg_catalog.pg_cast c
 WHERE c.oid>=16384 AND c.castcontext IN ('i','a') AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_extension e ON e.oid=d.refobjid JOIN pg_catalog.pg_namespace en ON en.oid=e.extnamespace
 WHERE d.classid='pg_catalog.pg_cast'::pg_catalog.regclass AND d.objid=c.oid AND d.refclassid='pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype='e' AND en.nspname=ANY($1))
 ORDER BY 1 LIMIT 101", &[&scope])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        if !casts.is_empty() {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                "Custom implicit/assignment casts exist in this database outside the extensions installed in the managed scope; DDL coercions could run unreviewed code. Drop the casts its objects field names, or make them explicit.",
            )
            .with_objects(&casts));
        }
        Ok(())
    }
    /// Gate checks on the live catalog. A check names only managed-schema
    /// relations, and reading a view reads what its rules depend on, so the
    /// conditions hold for every relation a check can reach: the managed
    /// schema's, and transitively every relation a view among them reads, in
    /// any schema. Objects no check can reach leave checks alone. `extra` and
    /// `outside` scope the coverage check, as in every inspection.
    ///
    /// A column's type may also be one in the managed scope, which the
    /// coverage check has just found covered: an enum, a domain or composite
    /// type over covered types, or an extension's type. Reading or comparing
    /// such a value runs builtin code or the extension's own (covered by its
    /// version), and so do the casts of an extension in the scope. A
    /// domain's constraints run when a value is converted to the domain. The
    /// check grammar writes no casts, but comparing a column with a literal
    /// converts the literal to the column's type. For a domain that is its
    /// base type, so the domain's own constraints never run. An array's
    /// elements, a range's bounds and a composite's attributes, though, go
    /// through their own types' input, domain constraints included.
    /// `CHECK_CONVERSION_CALLS` refuses, by name, a reachable column whose
    /// type holds such a domain below its top level (through a top-level
    /// domain's base type too), and a reachable view that converts values to
    /// a type holding one at any level or uses an operator, when the domain's
    /// constraints or the operator call a routine the history wrote or a
    /// volatile function. Plain domains and domains with builtin-only
    /// constraints (a regular expression, say) stay allowed.
    pub async fn check_purity(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
    ) -> Result<()> {
        let catalog = self.catalog(schema, extra, outside, "").await?;
        if !catalog.uncovered.is_empty() {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                "Live objects exceed contract purity coverage.",
            ));
        }
        // A check names only managed-schema relations ($1). Reading a view
        // reads the relations its rules depend on, in any schema, so `reach`
        // follows views from there, transitively.
        let safe = self.rows(r#"WITH RECURSIVE reach(rel) AS (
            SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1
            UNION
            SELECT d.refobjid FROM reach JOIN pg_catalog.pg_class v ON v.oid=reach.rel AND v.relkind='v'
            JOIN pg_catalog.pg_rewrite r ON r.ev_class=v.oid
            JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_rewrite'::regclass AND d.objid=r.oid AND d.refclassid='pg_catalog.pg_class'::regclass)
          SELECT NOT EXISTS (
          SELECT 1 FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
          WHERE c.oid IN (SELECT rel FROM reach) AND c.relkind='r'
          AND a.attnum>0 AND NOT a.attisdropped AND EXISTS(SELECT 1 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace tn ON tn.oid=t.typnamespace WHERE t.oid=a.atttypid AND tn.nspname<>'pg_catalog' AND tn.nspname<>ALL($2)))
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_cast k WHERE k.oid>=16384 AND k.castcontext IN ('i','a') AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_extension e ON e.oid=d.refobjid JOIN pg_catalog.pg_namespace en ON en.oid=e.extnamespace
            WHERE d.classid='pg_catalog.pg_cast'::pg_catalog.regclass AND d.objid=k.oid AND d.refclassid='pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype='e' AND en.nspname=ANY($2)))
          -- A composite type (relkind c) holds no rows to read.
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class c WHERE c.oid IN (SELECT rel FROM reach) AND c.relkind NOT IN ('r','p','i','I','S','v','c'))
          -- A check may read a view, and through it what the view reads.
          -- Views may only use builtin non-volatile functions, so reading one
          -- cannot write or take advisory locks.
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class v
            JOIN pg_catalog.pg_rewrite r ON r.ev_class=v.oid JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_rewrite'::regclass AND d.objid=r.oid
            JOIN pg_catalog.pg_proc p ON d.refclassid='pg_catalog.pg_proc'::regclass AND p.oid=d.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace
            WHERE v.oid IN (SELECT rel FROM reach) AND v.relkind='v' AND (pn.nspname<>'pg_catalog' OR p.provolatile='v'))"#, &[&schema, &scope(schema, extra)]).await?;
        if !safe[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                "Live types, casts or view functions exceed the verified builtin contract subset.",
            ));
        }
        let calls = self
            .rows(CHECK_CONVERSION_CALLS, &[&schema, &scope(schema, extra)])
            .await?;
        if !calls.is_empty() {
            let mut objects: Vec<String> = calls.iter().map(|r| r.get(0)).collect();
            objects.dedup();
            let mut reasons: Vec<String> = calls.iter().take(3).map(|r| r.get(1)).collect();
            if calls.len() > 3 {
                reasons.push("and more".into());
            }
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                &format!(
                    "Checks run no routine the history wrote and no volatile function, but the relations they can read could make them: {}. Give such a domain a constraint that uses only builtin non-volatile functions, or keep it at the top level of the column's type; a view must not convert values to it or use such an operator.",
                    reasons.join("; ")
                ),
            )
            .with_objects(&objects));
        }
        Ok(())
    }

    /// Execute captured check bytes once; consume at most two rows and inspect
    /// types without displaying values. False/NULL/shape/type/error/timeout fail.
    /// On any failure the caller rolls back or discards this dedicated session.
    pub async fn check(&self, sql: &str, schema: &str) -> Result<()> {
        use futures_util::TryStreamExt;
        if !super::analysis::contract_supported(sql, schema) {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                "Checks require managed-schema-qualified tables and supported builtin expressions.",
            ));
        }
        // Relations are explicitly qualified by the validated contract grammar.
        // Only builtin functions/operators participate in unqualified lookup.
        self.batch("SET LOCAL search_path=pg_catalog").await?;
        let passed = bounded(async {
            let rows = self
                .client
                .query_raw(
                    sql,
                    std::iter::empty::<&(dyn tokio_postgres::types::ToSql + Sync)>(),
                )
                .await?;
            tokio::pin!(rows);
            let first = rows.try_next().await?;
            let second = rows.try_next().await?;
            Ok(first.is_some_and(|row| {
                row.len() == 1
                    && row.columns()[0].type_() == &tokio_postgres::types::Type::BOOL
                    && row.try_get::<_, Option<bool>>(0).ok() == Some(Some(true))
            }) && second.is_none())
        })
        .await?;
        if !passed {
            return Err(Diagnostic::new("DB_CHECK_FAILED", "Contract requires exactly one row containing one non-NULL Boolean true; values suppressed."));
        }
        Ok(())
    }

    /// Shared execution primitive: consumes captured bytes only, within a caller-
    /// owned transaction. Replay and authorized migration use exactly this
    /// primitive, each migration in its own transaction that commits.
    /// `limit` bounds each statement and its deferred checks; the whole script
    /// gets `SERVER_LIMIT_MARGIN` more on the client. `extra` are the schemas
    /// the history creates, which belong to the managed scope.
    pub async fn execute_captured(
        &self,
        migration: &Migration,
        schema: &str,
        extra: &[String],
        limit: Duration,
    ) -> Result<()> {
        // A lone CREATE INDEX CONCURRENTLY replays as the equivalent plain
        // CREATE INDEX inside this transaction; the target runs it outside one.
        let concurrent = super::analysis::concurrent_index(&migration.sql, schema);
        if concurrent.is_some() && (!migration.requires.is_empty() || !migration.ensures.is_empty())
        {
            let mut d = Diagnostic::new("DB_REPLAY_UNSUPPORTED", "A CREATE INDEX CONCURRENTLY migration runs outside a transaction, so it cannot have requires/ensures checks. Put checks in a neighbouring migration.");
            d.source = Some(migration.name.clone());
            return Err(d);
        }
        let script = concurrent
            .as_ref()
            .map_or(migration.sql.as_str(), |c| c.replay_sql.as_str());
        if super::analysis::is_empty(&migration.sql, "postgresql") {
            let mut d = super::empty_migration();
            d.source = Some(migration.name.clone());
            return Err(d);
        }
        if !super::analysis::analyze(&migration.sql, schema).replay_supported {
            let mut d=Diagnostic::new("DB_REPLAY_UNSUPPORTED","Migration contains a statement that would end the migration transaction, change session identity or act outside this database; see analysis.unknowns.");
            d.source = Some(migration.name.clone());
            return Err(d);
        }
        // Statements on other schemas run as written. Ones that act on NTNT's
        // ledger schema or a system schema are refused before anything runs:
        // they can be no-ops in scratch yet act on the target.
        let protected = super::analysis::protected_targets(&migration.sql);
        if !protected.is_empty() {
            let statements = protected
                .iter()
                .map(|(number, name)| format!("statement {number} names {name}"))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(protected_schema(
                migration,
                &format!("Migration acts on _ntnt or a PostgreSQL system schema ({statements})."),
                protected.into_iter().map(|(_, name)| name).collect(),
            ));
        }
        // Migration SQL may call application routines; that is reviewed SQL.
        // A custom implicit/assignment cast planted elsewhere could instead run
        // unreviewed code during ordinary DDL coercion, so it still blocks.
        self.reject_custom_casts(&scope(schema, extra))
            .await
            .map_err(|mut d| {
                d.source = Some(migration.name.clone());
                d
            })?;
        // pg_catalog is implicitly searched first when omitted, while CREATE
        // uses the first explicit namespace. Never let app objects shadow builtins.
        // Inspection and checks use row_security=off so a policy can never
        // silently filter what they read. Authored scripts run as the
        // application's migrations always have: with normal row security, so
        // seed rows into FORCE ROW LEVEL SECURITY tables obey their policies.
        self.batch(&format!(
            "SET LOCAL search_path=\"{schema}\"; SET LOCAL row_security=on; SET LOCAL statement_timeout='{}s';",
            limit.as_secs()
        ))
        .await?;
        // Do not split on semicolons: PostgreSQL receives the captured full script.
        bounded_for(
            limit + SERVER_LIMIT_MARGIN,
            self.client.batch_execute(script),
        )
        .await
        .map_err(|mut d| {
            d.source = Some(migration.name.clone());
            d
        })?;
        // Fire deferred constraint checks now, attributed to this migration,
        // rather than at the commit that follows it (on the target and in
        // scratch replay alike).
        bounded_for(
            limit + SERVER_LIMIT_MARGIN,
            self.client.batch_execute("SET CONSTRAINTS ALL IMMEDIATE"),
        )
        .await
        .map_err(|mut d| {
            d.source = Some(migration.name.clone());
            d
        })?;
        // On the target a migration's commit ends its SET LOCAL values, and
        // its plain SETs would carry into the next migration's session. Drop
        // both here so every migration starts from the connection's settings,
        // in replay and on the target alike, then restore NTNT's own.
        self.batch(&format!("RESET ALL; {MIGRATION_SETTINGS}"))
            .await?;
        // Scratch was prepared without _ntnt. One a replayed migration created
        // (in a DO block, say) would let later statements that resolve there
        // replay while they act on the target's ledger.
        if self.scratch.get().is_some()
            && self
                .rows(
                    "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname='_ntnt')",
                    &[],
                )
                .await?[0]
                .get::<_, bool>(0)
        {
            return Err(protected_schema(
                migration,
                "Replaying this migration created the schema _ntnt.",
                vec!["schema:_ntnt".into()],
            ));
        }
        // The migration commits on scratch once it is inspected: refuse now
        // what cleanup could not undo (see `scratch`).
        if let Some(scope) = self.scratch.get() {
            self.refuse_lasting_effects(migration, scope).await?;
        }
        Ok(())
    }
}

/// Replace potentially sensitive deparsed definitions with identities before reporting.
/// Structural metadata remains available for object-level differences.
fn protect_definitions(value: &mut Value) {
    // Engine-deparsed SQL may itself contain credentials (for example a view
    // invoking a remote database). Keep exact-byte identities, never literals or
    // comments, in reports. Structural metadata and object-level diffs remain.
    if let Value::Object(map) = value {
        for (key, v) in map {
            if matches!(
                key.as_str(),
                "definition"
                    | "default"
                    | "labels"
                    | "using"
                    | "check"
                    | "comment"
                    | "options"
                    | "partition_key"
                    | "partition_bound"
            ) && !v.is_null()
            {
                *v = json!({"sha256":super::inventory::json_digest(v),"redacted":true});
            }
        }
    }
}
/// Normalize only typed owner/grant fields for the explicitly mapped logical owner.
/// Preserve other principals and privileges; never rewrite arbitrary SQL or strings.
fn canonical_roles(value: &mut Value, role: &str) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                if matches!(key.as_str(), "owner" | "grantor" | "grantee")
                    && v.as_str() == Some(role)
                {
                    *v = json!({"managed_role":true});
                } else {
                    canonical_roles(v, role);
                }
                if key == "grants" {
                    if let Value::Array(grants) = v {
                        grants.sort_by_key(Value::to_string);
                    }
                }
                // Policy role lists hold names, not owner/grant objects. A
                // policy created TO CURRENT_USER names scratch's role in the
                // plan and the target's role on execution.
                if key == "roles" {
                    if let Value::Array(roles) = v {
                        for r in roles.iter_mut() {
                            if r.as_str() == Some(role) {
                                *r = json!({"managed_role":true});
                            }
                        }
                        roles.sort_by_key(Value::to_string);
                    }
                }
            }
        }
        Value::Array(values) => {
            for v in values.iter_mut() {
                canonical_roles(v, role);
            }
        }
        _ => (),
    }
}
impl RelationalAdapter for Postgres {
    fn capabilities(version: i32) -> Value {
        json!({"adapter":"postgresql-catalog-v1", "canonicalization_version":1,"server_version_num":version,
            "operations": {"catalog_inspection":"supported_subset", "transactional_script_replay":"per_migration_commits_on_scratch", "target_ddl":"exact_plan_transactional_scripts", "sql_contracts":"closed_builtin_select_subset", "concurrent_indexes":"lone_named_create_index_concurrently", "baseline_adoption":"reviewed_prefix_checkpoint"},
            "inspection": {"tables_columns_constraints_indexes":"definition_fingerprints_and_validity", "schema_relation_column_routine_grants":"canonical_role_names", "views_sequences_routines_triggers_policies_partitions":"definition_fingerprints", "enums_domains_composite_types_operators_exclusion_constraints":"definition_fingerprints", "managed_schema_extensions":"version_covers_members", "range_and_base_types_external_dependencies_non_sql_routines":"unknown"},
            "guarantees": {"inspection_transaction":"repeatable_read_read_only", "execution_transaction":"read_committed_after_native_locks", "replay":"complete_transactional_scripts_in_empty_disposable_database", "atomic_target_ddl_checks_ledger":true}})
    }
    async fn catalog(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
    ) -> Result<Catalog> {
        self.batch("SET LOCAL search_path=pg_catalog").await?;
        let scope = scope(schema, extra);
        let mut catalog = Catalog::default();
        for (kind, sql) in super::catalog_sql::QUERIES {
            // $1, $2 and $3 as `catalog_sql` describes them.
            for r in self.rows(sql, &[&schema, &scope, &outside]).await? {
                let name: String = r.get(0);
                let mut value: Value = r.get(1);
                let supported: bool = r.get(2);
                canonical_roles(&mut value, role);
                protect_definitions(&mut value);
                // Keys in the managed schema stay unqualified; keys anywhere
                // else lead with the quoted schema name.
                let key = match r.get::<_, Option<String>>(3) {
                    None => format!("{kind}:{name}"),
                    Some(qualifier) => format!("{qualifier}.{kind}:{name}"),
                };
                if !supported {
                    catalog.uncovered.push(key.clone());
                }
                if catalog.objects.contains_key(&key) {
                    return Err(Diagnostic::new(
                        "DB_CATALOG_AMBIGUOUS",
                        "Two catalog objects share one key; inspection is incomplete.",
                    )
                    .with_objects(&[key]));
                }
                catalog.objects.insert(key, value);
            }
        }
        if !catalog.objects.contains_key(&format!("schema:{schema}")) {
            catalog.uncovered.push(format!("schema_missing:{schema}"));
        }
        catalog.uncovered.sort();
        catalog.uncovered.dedup();
        Ok(catalog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grants_ignore_catalog_oid_order_but_preserve_privileges() {
        let mut a = json!({"grants":[{"grantee":"a","grantor":"a","privilege":"SELECT"},{"grantee":"PUBLIC","grantor":"a","privilege":"SELECT"}]});
        let mut b = json!({"grants":[{"grantee":"PUBLIC","grantor":"b","privilege":"SELECT"},{"grantee":"b","grantor":"b","privilege":"SELECT"}]});
        canonical_roles(&mut a, "a");
        canonical_roles(&mut b, "b");
        assert_eq!(a, b);
        b["grants"][0]["privilege"] = json!("DELETE");
        assert_ne!(a, b);
    }
    fn fixture_endpoint() -> Endpoint {
        Endpoint {
            connection_secret: "FIXTURE".into(),
            database: "app".into(),
            role: "app".into(),
        }
    }

    #[test]
    fn tls_modes_never_fall_back_to_plaintext() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        /// Only a hang reaches this: the client closes as soon as it reads 'N'.
        const HANG_GUARD: Duration = Duration::from_secs(60);
        // verify-ca needs a CA file. Any CA will do: no handshake starts below.
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("ca.pem");
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        std::fs::write(&ca, params.self_signed(&key).unwrap().pem()).unwrap();
        // A backslash escapes in key/value secrets, so use forward slashes.
        let ca = ca.to_str().unwrap().replace('\\', "/");
        // A server without TLS answers PostgreSQL's SSLRequest with 'N'. Every TLS
        // mode must stop there rather than continue with a plaintext startup.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for tls in [
                "sslmode=require".to_string(),
                format!("sslmode=verify-ca sslrootcert='{ca}'"),
                "sslmode=verify-full".to_string(),
            ] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = [0u8; 8];
                    socket.read_exact(&mut request).await.unwrap();
                    socket.write_all(b"N").await.unwrap();
                    // Anything sent before the client closes the connection would
                    // be a plaintext StartupMessage.
                    let mut rest = Vec::new();
                    let _ = tokio::time::timeout(HANG_GUARD, socket.read_to_end(&mut rest))
                        .await
                        .expect("the client kept the connection open after 'N'");
                    (request, rest)
                });
                let secret = crate::interpreter::SecretValue::new(
                    "FIXTURE",
                    format!("host=127.0.0.1 port={port} user=app dbname=app password=fixture-only {tls}"),
                )
                .unwrap();
                let error = Postgres::connect(&secret, &fixture_endpoint())
                    .await
                    .err()
                    .expect("a TLS mode must not connect without TLS");
                assert_eq!(error.code, "DB_TLS", "{tls}: {error}");
                assert!(error.message.contains("No plaintext fallback"), "{error}");
                let (request, rest) = server.await.unwrap();
                // SSLRequest: length 8, request code 80877103.
                assert_eq!(request, [0, 0, 0, 8, 4, 210, 22, 47], "{tls}");
                assert!(rest.is_empty(), "{tls}: client continued in plaintext: {rest:?}");
            }
        });
    }

    #[test]
    fn tls_needs_tcp_and_no_unsupported_channel_binding() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut cases = vec![
            "host=127.0.0.1 user=app dbname=app sslmode=verify-full channel_binding=require",
            "host=127.0.0.1 user=app dbname=app",
            "host=127.0.0.1 user=app dbname=app sslmode=prefer",
            "host=127.0.0.1 user=other dbname=app sslmode=verify-full",
        ];
        #[cfg(unix)]
        cases.push("host=/var/run/postgresql user=app dbname=app sslmode=verify-full");
        runtime.block_on(async {
            for case in cases {
                let secret =
                    crate::interpreter::SecretValue::new("FIXTURE", case.to_string()).unwrap();
                let error = Postgres::connect(&secret, &fixture_endpoint())
                    .await
                    .err()
                    .expect("rejected before connecting");
                assert_eq!(error.code, "DB_CONNECTION_CONFIG", "{case}: {error}");
            }
        });
    }

    #[test]
    fn capabilities_report_concurrent_index_builds_as_supported() {
        // Shipped in 0.6.1 (ntnt db reconcile); reports must not call it unimplemented.
        let capabilities = Postgres::capabilities(170_000);
        assert_eq!(
            capabilities["operations"]["concurrent_indexes"],
            "lone_named_create_index_concurrently"
        );
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
    fn target_transaction_is_really_read_only_even_for_superuser() {
        let url =
            std::env::var("NTNT_DB_TEST_ADMIN").expect("explicit disposable fixture URL required");
        let config: Config = url.parse().unwrap();
        let endpoint = Endpoint {
            connection_secret: "FIXTURE".into(),
            database: config.get_dbname().unwrap().into(),
            role: config.get_user().unwrap().into(),
        };
        let secret = crate::interpreter::SecretValue::new("FIXTURE", url).unwrap();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let pg=Postgres::connect(&secret,&endpoint).await.unwrap();
            pg.begin_readonly().await.unwrap();
            let r=pg.rows("SELECT current_setting('transaction_read_only'),current_setting('transaction_isolation')",&[]).await.unwrap();
            assert_eq!(r[0].get::<_,String>(0),"on");
            assert_eq!(r[0].get::<_,String>(1),"repeatable read");
            let failure=pg.batch("CREATE TABLE public.ntnt245_readonly_canary(id integer)").await.unwrap_err();
            assert!(failure.message.contains("25006"),"{failure}");
            pg.rollback().await.unwrap();
            let exists=pg.rows("SELECT to_regclass('public.ntnt245_readonly_canary') IS NOT NULL",&[]).await.unwrap();
            assert!(!exists[0].get::<_,bool>(0));
        });
    }
}
