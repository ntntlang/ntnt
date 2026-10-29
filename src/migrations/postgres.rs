//! Dedicated physical PostgreSQL sessions, bounded I/O and trusted catalog SQL.
//! Authored DDL/checks require closed validation; inspection uses only fixed catalog SQL.
use super::inventory::{Endpoint, Migration};
use super::{Diagnostic, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;
use tokio_postgres::{Client, Config, NoTls, Row};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Apply the per-exchange deadline and expose only SQLSTATE, never server error payloads.
/// The owning adapter must discard its dedicated connection after cancellation/errors.
pub(crate) async fn bounded<T>(
    future: impl Future<Output = std::result::Result<T, tokio_postgres::Error>>,
) -> Result<T> {
    match tokio::time::timeout(IO_TIMEOUT, future).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Diagnostic::new(
            "DB_POSTGRES",
            &format!(
                "PostgreSQL operation failed (SQLSTATE {}). Server details suppressed.",
                e.code().map(|c| c.code()).unwrap_or("unavailable")
            ),
        )),
        Err(_) => Err(Diagnostic::new(
            "DB_TIMEOUT",
            "Database connect/authentication/query deadline exceeded.",
        )),
    }
}

pub(crate) struct Postgres {
    client: Client,
    task: tokio::task::JoinHandle<()>,
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
}
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Catalog {
    pub objects: BTreeMap<String, Value>,
    pub uncovered: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HistoryEntry {
    pub position: i64,
    pub id: String,
    pub name: String,
    pub checksum: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct History {
    pub present: bool,
    pub entries: Vec<HistoryEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Value>,
}

/// Small relational boundary: each adapter owns its native catalog and operation
/// capabilities. A driver alone does not confer migration support.
pub(crate) trait RelationalAdapter {
    async fn catalog(&self, schema: &str, role: &str) -> Result<Catalog>;
    fn capabilities(version: i32) -> Value;
}
impl Postgres {
    /// Open a dedicated session after checking explicit host, role, database and transport.
    /// The initial connection is read-only by default; this adapter accepts only explicit
    /// NoTLS configuration on trusted transport and never silently downgrades TLS.
    pub async fn connect(
        secret: &crate::interpreter::SecretValue,
        endpoint: &Endpoint,
    ) -> Result<Self> {
        let mut config: Config = secret.expose().parse().map_err(|_| {
            Diagnostic::new(
                "DB_CONNECTION_CONFIG",
                "Invalid connection secret; details suppressed.",
            )
        })?;
        if config.get_dbname() != Some(&endpoint.database)
            || config.get_user() != Some(&endpoint.role)
            || config.get_hosts().len() != 1
            || config.get_options().is_some()
            || config.get_ssl_mode() != tokio_postgres::config::SslMode::Disable
        {
            return Err(Diagnostic::new("DB_CONNECTION_CONFIG","Use one explicit host, the declared database/role, sslmode=disable on a trusted local transport, and no startup options. TLS transport is not yet supported."));
        }
        config.connect_timeout(IO_TIMEOUT);
        config.application_name("ntnt-db-inspect-v1");
        config.options("-c default_transaction_read_only=on -c search_path=pg_catalog -c statement_timeout=5000 -c lock_timeout=2000 -c idle_in_transaction_session_timeout=10000 -c standard_conforming_strings=on -c row_security=off");
        let (client, connection) = bounded(config.connect(NoTls)).await?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(Self { client, task })
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
        let identity = Identity {
            cluster: r.get(0),
            database_oid: r.get(1),
            database: r.get(2),
            role: r.get(3),
            version: r.get(4),
            settings: r.get(5),
            authority,
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
        let ledger_scope = self.catalog("_ntnt", "").await.map_err(|_| bad())?;
        if ledger_scope.uncovered.iter().any(|key| {
            ledger_scope
                .objects
                .get(key)
                .is_none_or(|object| object["external"] != true)
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
    /// Acquire database-local ownership of the single reserved ledger namespace.
    /// This session lock survives per-migration commits. Drop discards the socket;
    /// it cannot leak into a pool. PostgreSQL lock_timeout bounds competing owners.
    pub async fn own_migrations(&self) -> Result<()> {
        self.rows("SELECT pg_catalog.pg_advisory_lock(1853124212, 245)", &[])
            .await?;
        Ok(())
    }

    /// Native locks protect data assumptions from ordinary writers, unlike advisory
    /// ownership. READ COMMITTED takes the validation snapshot *after* these locks.
    /// Administrators and schema-owner credentials remain trusted, not sandboxed.
    pub async fn begin_migration(&self, schema: &str) -> Result<()> {
        self.batch("BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE; SET LOCAL synchronous_commit=on; SET LOCAL search_path=pg_catalog; SET LOCAL row_security=off; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces='';").await?;
        let tables = self.rows("SELECT pg_catalog.quote_ident(n.nspname)||'.'||pg_catalog.quote_ident(c.relname) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE (n.nspname=$1 OR n.nspname='_ntnt') AND c.relkind='r' ORDER BY n.nspname,c.relname", &[&schema]).await?;
        for row in tables {
            self.batch(&format!(
                "LOCK TABLE ONLY {} IN ACCESS EXCLUSIVE MODE",
                row.get::<_, String>(0)
            ))
            .await?;
        }
        Ok(())
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
          WHERE n.nspname='_ntnt' AND c.relname='schema_migrations_v2'"#, &[]).await?;
        if safe.len() != 1 || !safe[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_LEDGER_UNSAFE",
                "Ledger is not a supported mutation-safe owned heap; no write was attempted.",
            ));
        }
        Ok(history)
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
        if !present {
            self.batch("CREATE SCHEMA _ntnt; CREATE TABLE _ntnt.schema_migrations_v2 (position bigint NOT NULL PRIMARY KEY,id text NOT NULL,name text NOT NULL,checksum text NOT NULL,state text NOT NULL,logical_database text NOT NULL,managed_schema text NOT NULL)").await?;
        }
        self.rows("INSERT INTO _ntnt.schema_migrations_v2(position,id,name,checksum,state,logical_database,managed_schema) VALUES($1,$2,$3,$4,'applied',$5,$6)", &[&(position as i64), &migration.id.to_string(), &migration.name, &migration.checksum, &database, &schema]).await?;
        Ok(())
    }

    /// Validate isolation, compatible settings and restricted ownership before creating scope.
    /// On success a rollback-only transaction remains open. The caller must roll back on
    /// every success/error exit or discard the session; no reset/adoption is performed.
    pub async fn prepare_scratch(
        &self,
        target: &Identity,
        endpoint: &Endpoint,
        schema: &str,
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
        self.batch("BEGIN ISOLATION LEVEL REPEATABLE READ READ WRITE; SET LOCAL search_path=pg_catalog; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces='';").await?;
        let r=bounded(self.client.query_one(r#"
SELECT NOT (r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls)
 AND d.datdba=r.oid AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members WHERE member=r.oid)
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_event_trigger)
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname<>'plpgsql')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_default_acl)
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname NOT IN ('public','information_schema') AND nspname NOT LIKE 'pg_%')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public')
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='public')
FROM pg_catalog.pg_roles r,pg_catalog.pg_database d WHERE r.rolname=current_user AND d.datname=pg_catalog.current_database()
"#,&[])).await?;
        if !r.get::<_, bool>(0) {
            return Err(Diagnostic::new("DB_SCRATCH_NOT_EMPTY","Scratch must be an empty disposable database owned by an unprivileged, non-member role, with no user objects, event triggers, extensions or default grants."));
        }
        let public = self.catalog("public", &endpoint.role).await?;
        if public.objects.len() != 1
            || !public.objects.contains_key("schema:public")
            || !public.uncovered.is_empty()
        {
            return Err(Diagnostic::new("DB_SCRATCH_NOT_EMPTY","Scratch public namespace contains unsupported objects or cannot be fully inspected."));
        }
        self.batch(&format!(
            "CREATE SCHEMA \"{schema}\"; SET LOCAL search_path=pg_catalog,\"{schema}\";"
        ))
        .await?;
        Ok(identity)
    }
    /// Reject unsupported live invocation paths before preparing any authored
    /// check. Names alone and provolatile are not proofs: only builtin base types,
    /// heap tables, builtin indexes/operators and the closed grammar are admitted.
    /// Custom implicit/assignment casts are rejected database-wide, conservatively.
    pub async fn check_purity(&self, schema: &str) -> Result<()> {
        let catalog = self.catalog(schema, "").await?;
        if !catalog.uncovered.is_empty() {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                "Live objects exceed contract purity coverage.",
            ));
        }
        let safe = self.rows(r#"SELECT NOT EXISTS (
          SELECT 1 FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
          JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind='r'
          AND a.attnum>0 AND NOT a.attisdropped AND a.atttypid NOT IN (16,17,20,21,23,25,700,701,1043,1082,1114,1184,1700,2950))
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_cast WHERE oid>=16384 AND castcontext IN ('i','a'))
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind NOT IN ('r','i'))"#, &[&schema]).await?;
        if !safe[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                "Live types or casts exceed the verified builtin contract subset.",
            ));
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
    /// owned transaction. Replay and authorized migration use exactly this primitive.
    pub async fn execute_captured(&self, migration: &Migration, schema: &str) -> Result<()> {
        if !super::analysis::analyze(&migration.sql, schema).replay_supported {
            let mut d=Diagnostic::new("DB_REPLAY_UNSUPPORTED","Migration is outside the supported replay subset; no arbitrary SQL sandbox is claimed.");
            d.source = Some(migration.name.clone());
            return Err(d);
        }
        // DDL coercions can invoke custom assignment/implicit casts too. Verify
        // native resolution before both scratch replay and authorized execution.
        self.check_purity(schema).await?;
        // pg_catalog is implicitly searched first when omitted, while CREATE
        // uses the first explicit namespace. Never let app objects shadow builtins.
        self.batch(&format!("SET LOCAL search_path=\"{schema}\";"))
            .await?;
        // Do not split on semicolons: PostgreSQL receives the captured full script.
        self.batch(&migration.sql).await.map_err(|mut d| {
            d.source = Some(migration.name.clone());
            d
        })
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
                "definition" | "default" | "using" | "check" | "comment" | "options"
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
            "operations": {"catalog_inspection":"supported_subset", "restricted_ddl_replay":"rollback_only_scratch", "target_ddl":"exact_plan_transactional_subset", "sql_contracts":"closed_builtin_select_subset", "concurrent_indexes":"not_implemented", "baseline_adoption":"not_implemented"},
            "inspection": {"tables_columns_constraints_indexes":"definition_fingerprints_and_validity", "schema_relation_column_grants":"canonical_role_names", "views_sequences_routines_triggers_policies":"inventoried_opaque", "partitioning_extensions_custom_types_external_dependencies":"unknown"},
            "guarantees": {"inspection_transaction":"repeatable_read_read_only", "execution_transaction":"read_committed_after_native_locks", "replay":"restricted_subset_in_empty_disposable_database", "atomic_target_ddl_checks_ledger":true}})
    }
    async fn catalog(&self, schema: &str, role: &str) -> Result<Catalog> {
        self.batch("SET LOCAL search_path=pg_catalog").await?;
        let mut catalog = Catalog::default();
        for (kind, sql) in super::catalog_sql::QUERIES {
            for r in self.rows(sql, &[&schema]).await? {
                let name: String = r.get(0);
                let mut value: Value = r.get(1);
                let supported: bool = r.get(2);
                canonical_roles(&mut value, role);
                protect_definitions(&mut value);
                let key = format!("{kind}:{name}");
                if !supported {
                    catalog.uncovered.push(key.clone());
                }
                if catalog.objects.insert(key, value).is_some() {
                    return Err(Diagnostic::new(
                        "DB_CATALOG_AMBIGUOUS",
                        "Catalog object keys collided; inspection is incomplete.",
                    ));
                }
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
