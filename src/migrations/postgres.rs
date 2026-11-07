//! Dedicated physical PostgreSQL sessions, bounded I/O and trusted catalog SQL.
//! Authored DDL/checks require closed validation; inspection uses only fixed catalog SQL.
use super::inventory::{Endpoint, Migration};
use super::{Diagnostic, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
use std::time::Duration;
use tokio_postgres::{Client, Config, NoTls, Row};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Apply the per-exchange deadline and expose only SQLSTATE, never server error payloads.
/// The owning adapter must discard its dedicated connection after cancellation/errors.
pub(crate) async fn bounded<T>(
    future: impl Future<Output = std::result::Result<T, tokio_postgres::Error>>,
) -> Result<T> {
    bounded_for(IO_TIMEOUT, future).await
}
/// Authored migration scripts may run much longer than catalog queries (index
/// builds, rewrites). Kept inside migrate's 100-second work deadline for now.
pub(crate) const SCRIPT_TIMEOUT: Duration = Duration::from_secs(90);
/// Transaction settings NTNT relies on while inspecting, replaying and
/// executing. Reapplied after each migration's RESET ALL.
const MIGRATION_SETTINGS: &str = "SET LOCAL synchronous_commit=on; SET LOCAL search_path=pg_catalog; SET LOCAL row_security=off; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces=''; SET LOCAL lock_timeout='1s'; SET LOCAL statement_timeout=5000;";
async fn bounded_for<T>(
    limit: Duration,
    future: impl Future<Output = std::result::Result<T, tokio_postgres::Error>>,
) -> Result<T> {
    match tokio::time::timeout(limit, future).await {
        Ok(Ok(v)) => Ok(v),
        // A lock wait that hit lock_timeout, or a deadlock victim. Nothing was
        // committed; the caller may retry the whole migration transaction.
        Ok(Err(e)) if matches!(e.code().map(|c| c.code()), Some("55P03") | Some("40P01")) => {
            Err(Diagnostic::new(
                "DB_LOCK_CONFLICT",
                &format!(
                    "A lock was not granted in time (SQLSTATE {}).",
                    e.code().map(|c| c.code()).unwrap_or("unavailable")
                ),
            ))
        }
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

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
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
use super::model::{Catalog, History, HistoryEntry};

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
    pub async fn validate_execution_scope(&self, schema: &str) -> Result<()> {
        // A cross-scope dependent is not an owned object. Even non-cascading changes
        // are conservatively rejected rather than promising a transitive effect bound.
        let incoming = self.rows("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.confrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_class f ON f.oid=k.conrelid JOIN pg_catalog.pg_namespace fn ON fn.oid=f.relnamespace WHERE n.nspname=$1 AND fn.nspname<>$1)", &[&schema]).await?;
        if incoming[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_EXTERNAL_DEPENDENCY",
                "Incoming foreign keys from outside the managed schema block execution.",
            ));
        }
        Ok(())
    }

    /// Acquire database-local ownership of the single reserved ledger namespace.
    /// This session lock survives per-migration commits. Drop discards the socket;
    /// it cannot leak into a pool. PostgreSQL lock_timeout bounds competing owners.
    /// A second migrator waits a few seconds for the first, then gives up.
    pub async fn own_migrations(&self) -> Result<()> {
        self.batch("SET lock_timeout = '5s'").await?;
        let owned = self
            .rows("SELECT pg_catalog.pg_advisory_lock(1853124212, 245)", &[])
            .await;
        self.batch("SET lock_timeout = '2s'").await?;
        owned.map(|_| ())
    }

    /// The advisory lock already serializes migrators. Each statement takes only
    /// the locks PostgreSQL requires for it, and a lock that is not granted
    /// within one second fails the attempt instead of queueing application
    /// queries behind it; the caller retries the whole transaction.
    pub async fn begin_migration(&self, _schema: &str) -> Result<()> {
        self.batch("BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE; SET LOCAL synchronous_commit=on; SET LOCAL search_path=pg_catalog; SET LOCAL row_security=off; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces=''; SET LOCAL lock_timeout='1s';").await
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
        self.batch("BEGIN ISOLATION LEVEL REPEATABLE READ READ WRITE; SET LOCAL search_path=pg_catalog; SET LOCAL default_table_access_method=heap; SET LOCAL default_tablespace=''; SET LOCAL temp_tablespaces='';").await?;
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
"#,&[&target_bypasses_rls])).await?;
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
        // `public` already exists (and was just verified empty); any other
        // managed schema is created inside this rollback-only transaction.
        let create = if schema == "public" {
            String::new()
        } else {
            format!("CREATE SCHEMA \"{schema}\"; ")
        };
        self.batch(&format!(
            "{create}SET LOCAL search_path=pg_catalog,\"{schema}\";"
        ))
        .await?;
        Ok(identity)
    }
    /// Reject unsupported live invocation paths before preparing any authored
    /// check. Names alone and provolatile are not proofs: only builtin base types,
    /// heap tables, builtin indexes/operators and the closed grammar are admitted.
    /// Custom implicit/assignment casts are rejected database-wide, conservatively.
    async fn reject_custom_casts(&self) -> Result<()> {
        let safe = self
            .rows("SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_cast WHERE oid>=16384 AND castcontext IN ('i','a'))", &[])
            .await?;
        if !safe[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                "Custom implicit/assignment casts exist in this database; DDL coercions could run unreviewed code.",
            ));
        }
        Ok(())
    }
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
          AND a.attnum>0 AND NOT a.attisdropped AND EXISTS(SELECT 1 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace tn ON tn.oid=t.typnamespace WHERE t.oid=a.atttypid AND tn.nspname<>'pg_catalog'))
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_cast WHERE oid>=16384 AND castcontext IN ('i','a'))
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind NOT IN ('r','p','i','I','S','v'))
          -- A check may read a view. Views may only use builtin non-volatile
          -- functions, so reading one cannot write or take advisory locks.
          AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class v JOIN pg_catalog.pg_namespace n ON n.oid=v.relnamespace
            JOIN pg_catalog.pg_rewrite r ON r.ev_class=v.oid JOIN pg_catalog.pg_depend d ON d.classid='pg_catalog.pg_rewrite'::regclass AND d.objid=r.oid
            JOIN pg_catalog.pg_proc p ON d.refclassid='pg_catalog.pg_proc'::regclass AND p.oid=d.refobjid JOIN pg_catalog.pg_namespace pn ON pn.oid=p.pronamespace
            WHERE n.nspname=$1 AND v.relkind='v' AND (pn.nspname<>'pg_catalog' OR p.provolatile='v'))"#, &[&schema]).await?;
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
            let mut d=Diagnostic::new("DB_REPLAY_UNSUPPORTED","Migration contains a statement that would end the migration transaction, change session identity or act outside this database; see analysis.unknowns.");
            d.source = Some(migration.name.clone());
            return Err(d);
        }
        // Replay runs several migrations in one transaction. If an earlier one
        // ran here, constraint modes can only be restored when same-named
        // constraints agree on their default: SET CONSTRAINTS name affects all.
        let continues = self
            .rows("SELECT coalesce(pg_catalog.current_setting('ntnt.migration_boundary', true), '') = 'on'", &[])
            .await?[0]
            .get::<_, bool>(0);
        if continues && self.mixed_deferral().await? {
            let mut d = Diagnostic::new("DB_REPLAY_UNSUPPORTED", "Constraints sharing a name in one schema have different INITIALLY DEFERRED defaults, so replay cannot reproduce this migration's fresh-transaction constraint modes. Give the constraints distinct names.");
            d.source = Some(migration.name.clone());
            return Err(d);
        }
        // Migration SQL may call application routines; that is reviewed SQL.
        // A custom implicit/assignment cast planted elsewhere could instead run
        // unreviewed code during ordinary DDL coercion, so it still blocks.
        self.reject_custom_casts().await?;
        // pg_catalog is implicitly searched first when omitted, while CREATE
        // uses the first explicit namespace. Never let app objects shadow builtins.
        // Inspection and checks use row_security=off so a policy can never
        // silently filter what they read. Authored scripts run as the
        // application's migrations always have: with normal row security, so
        // seed rows into FORCE ROW LEVEL SECURITY tables obey their policies.
        self.batch(&format!(
            "SET LOCAL search_path=\"{schema}\"; SET LOCAL row_security=on; SET LOCAL statement_timeout='{}s';",
            SCRIPT_TIMEOUT.as_secs()
        ))
        .await?;
        // Do not split on semicolons: PostgreSQL receives the captured full script.
        bounded_for(
            SCRIPT_TIMEOUT + Duration::from_secs(5),
            self.client.batch_execute(&migration.sql),
        )
        .await
        .map_err(|mut d| {
            d.source = Some(migration.name.clone());
            d
        })?;
        // End each migration as its own commit would: fire deferred constraint
        // checks now, attributed to this migration. Replay runs many migrations
        // in one transaction, and pending trigger events would otherwise block
        // the next migration's ALTER TABLE.
        bounded_for(
            SCRIPT_TIMEOUT + Duration::from_secs(5),
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
        self.batch(&format!(
            "RESET ALL; {MIGRATION_SETTINGS} SET LOCAL ntnt.migration_boundary='on';"
        ))
        .await?;
        // PostgreSQL has no "reset to default" for constraint modes. Restore
        // INITIALLY DEFERRED constraints so the next migration starts as a
        // new transaction would. Names with mixed defaults are refused when
        // the next migration starts (see above).
        let deferred: Vec<String> = self
            .rows("SELECT pg_catalog.quote_ident(n.nspname)||'.'||pg_catalog.quote_ident(k.conname) FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_namespace n ON n.oid=k.connamespace GROUP BY n.nspname,k.conname HAVING pg_catalog.bool_and(k.condeferred) ORDER BY 1", &[])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        if !deferred.is_empty() {
            self.batch(&format!("SET CONSTRAINTS {} DEFERRED", deferred.join(",")))
                .await?;
        }
        Ok(())
    }

    async fn mixed_deferral(&self) -> Result<bool> {
        Ok(self
            .rows("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint GROUP BY connamespace,conname HAVING pg_catalog.bool_or(condeferred) <> pg_catalog.bool_and(condeferred))", &[])
            .await?[0]
            .get(0))
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
            "operations": {"catalog_inspection":"supported_subset", "transactional_script_replay":"rollback_only_scratch", "target_ddl":"exact_plan_transactional_scripts", "sql_contracts":"closed_builtin_select_subset", "concurrent_indexes":"not_implemented", "baseline_adoption":"reviewed_prefix_checkpoint"},
            "inspection": {"tables_columns_constraints_indexes":"definition_fingerprints_and_validity", "schema_relation_column_routine_grants":"canonical_role_names", "views_sequences_routines_triggers_policies_partitions":"definition_fingerprints", "managed_schema_extensions":"version", "custom_types_external_dependencies_non_sql_routines":"unknown"},
            "guarantees": {"inspection_transaction":"repeatable_read_read_only", "execution_transaction":"read_committed_after_native_locks", "replay":"complete_transactional_scripts_in_empty_disposable_database", "atomic_target_ddl_checks_ledger":true}})
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
