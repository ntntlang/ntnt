//! Scratch replay's lifecycle on PostgreSQL (#430). Each replayed migration
//! commits on scratch in a transaction set up like the target's, so scratch
//! is cleaned up afterwards, and only what NTNT can prove it created is
//! ever dropped:
//!
//! - A session advisory lock serializes replays on one scratch database,
//!   from the first look at its contents until cleanup ends.
//! - Before the first migration a committed marker, the schema
//!   `_ntnt_replay` with one row, records the replay and the state of
//!   `public`, which the scratch role can change (as a member of
//!   `pg_database_owner`) but does not own.
//! - Cleanup drops everything the scratch role owns (`DROP OWNED`), restores
//!   `public` and runs the emptiness check again, in one transaction.
//! - A marker that a replay which never finished left behind is cleaned
//!   the same way, but only when no other role owns anything in scratch.
//! - Changes `DROP OWNED` cannot undo (database and role settings, grants
//!   and comments on the database, a password, objects given to another
//!   role) are refused before the migration that made them commits.
use super::super::inventory::{Limits, Migration};
use super::super::{Diagnostic, Result};
use super::{
    bounded_for, protected_schema, quote, Postgres, ScratchScope, MIGRATION_TRANSACTION,
    SERVER_LIMIT_MARGIN,
};
use serde_json::{json, Value};
use std::time::Duration;

/// The replay lock: a session advisory lock on the scratch database. The
/// target's migrator lock uses the same first key with 245.
const LOCK: &str = "SELECT pg_catalog.pg_advisory_lock(1853124212, 430)";
const UNLOCK: &str = "SELECT pg_catalog.pg_advisory_unlock(1853124212, 430)";

/// The marker's schema and table. Its one row names the replay and holds
/// the state `public` had before the first migration, which cleanup
/// restores, also after a replay that never finished.
pub(super) const MARKER_SCHEMA: &str = "_ntnt_replay";
const CREATE_MARKER: &str = "CREATE SCHEMA _ntnt_replay; CREATE TABLE _ntnt_replay.replay_v1 (id text, started_at timestamptz, public_oid oid, public_owner text, public_acl text, public_comment text);";

/// The marker's shape: `_ntnt_replay` and `replay_v1` owned by this role,
/// a plain table whose reads run no code (no rules, triggers or row
/// security), with exactly these columns, and nothing else in the schema.
/// A table's row type depends on the table, not the schema.
const MARKER_SHAPE: &str = r#"SELECT n.oid, c.oid, n.nspowner=me.oid AND c.relowner=me.oid AND c.relkind='r' AND c.relpersistence='p'
 AND NOT c.relhasrules AND NOT c.relhastriggers AND NOT c.relrowsecurity
 AND (SELECT pg_catalog.count(*) FROM pg_catalog.pg_depend d WHERE d.refclassid='pg_catalog.pg_namespace'::pg_catalog.regclass AND d.refobjid=n.oid)=1
 AND (SELECT pg_catalog.array_agg(a.attname::text||':'||a.atttypid::pg_catalog.regtype::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped)
  = ARRAY['id:text','started_at:timestamp with time zone','public_oid:oid','public_owner:text','public_acl:text','public_comment:text']
FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles me ON me.rolname=session_user
LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid AND c.relname='replay_v1'
WHERE n.nspname='_ntnt_replay'"#;
const MARKER_ROW: &str = r#"SELECT id, pg_catalog.to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'), public_oid, public_owner, public_acl, public_comment
FROM ONLY _ntnt_replay.replay_v1 LIMIT 2"#;

/// `public` as cleanup restores it: its owner, privileges and comment.
const PUBLIC_STATE: &str = "SELECT n.oid, pg_catalog.pg_get_userbyid(n.nspowner)::text, n.nspacl::text, pg_catalog.obj_description(n.oid, 'pg_namespace')::text FROM pg_catalog.pg_namespace n WHERE n.nspname='public'";

/// Privileges `public` has ($1, as `nspacl::text`, with owner $2) but not
/// now, or now but not then, as sets of (grantor, grantee, privilege, grant
/// option); NULL means the owner's default privileges.
const PUBLIC_GRANTS: &str = r#"WITH want AS (SELECT a.grantor, a.grantee, a.privilege_type, a.is_grantable
  FROM pg_catalog.aclexplode(coalesce($1::text::pg_catalog.aclitem[], pg_catalog.acldefault('n', (SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=$2)))) a),
have AS (SELECT a.grantor, a.grantee, a.privilege_type, a.is_grantable FROM pg_catalog.pg_namespace n
  CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(n.nspacl, pg_catalog.acldefault('n', n.nspowner))) a WHERE n.nspname='public')"#;

/// Statements that give `public` its privileges back: revoke what it holds
/// now, then grant what it held. Run by the owner or a member of it, each
/// grant records the owner as grantor, as `public`'s grants normally do.
/// None when the privileges are already the same.
const RESTORE_PUBLIC_GRANTS: &str = r#"SELECT s FROM (
  SELECT 1 AS o, 'REVOKE ALL ON SCHEMA public FROM '||pg_catalog.string_agg(DISTINCT CASE WHEN h.grantee=0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(h.grantee)) END, ', ')||' CASCADE' AS s FROM have h
  UNION ALL
  SELECT 2, 'GRANT '||pg_catalog.string_agg(w.privilege_type, ', ' ORDER BY w.privilege_type)||' ON SCHEMA public TO '||CASE WHEN w.grantee=0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantee)) END||CASE WHEN w.is_grantable THEN ' WITH GRANT OPTION' ELSE '' END
  FROM want w GROUP BY w.grantee, w.is_grantable) x
WHERE s IS NOT NULL AND (EXISTS (SELECT * FROM want EXCEPT SELECT * FROM have) OR EXISTS (SELECT * FROM have EXCEPT SELECT * FROM want))
ORDER BY o, s"#;

/// Casts have no owner, and `DROP OWNED` refuses to drop a type one
/// depends on: drop the casts of the scratch role's types first. An
/// extension's casts go with the extension.
const DROP_CASTS: &str = r#"SELECT pg_catalog.format('DROP CAST (%s AS %s)', c.castsource::pg_catalog.regtype, c.casttarget::pg_catalog.regtype)
FROM pg_catalog.pg_cast c WHERE c.oid>=16384
 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d WHERE d.classid='pg_catalog.pg_cast'::pg_catalog.regclass AND d.objid=c.oid AND d.deptype='e')
 AND EXISTS (SELECT 1 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_roles me ON me.rolname=session_user WHERE t.oid IN (c.castsource, c.casttarget) AND t.typowner=me.oid)
ORDER BY 1"#;

/// What scratch holds that cleanup would leave or must not touch: objects
/// of this database another role owns, outside PostgreSQL's schemas (as in
/// the emptiness check) and the temporary schemas of sessions, and casts
/// and other ownerless objects. An object belongs to
/// the scratch role when it or what it is part of (by an internal,
/// automatic or extension dependency, followed up) is the scratch role's:
/// a trusted extension's members are owned by the bootstrap superuser, and
/// so are a range type's constructor functions. A cast belongs to it when
/// it casts one of its types. `public` ($1, by OID) is restored instead.
/// Objects `initdb` created have OIDs below 16384.
const FOREIGN_OBJECTS: &str = r#"WITH RECURSIVE me(oid) AS (SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=session_user),
temp(oid) AS (SELECT n.oid FROM pg_catalog.pg_namespace n WHERE n.nspname ~ '^pg_(toast_)?temp_[0-9]+$' OR n.nspname IN ('pg_catalog','pg_toast','information_schema')),
owned(classid, objid, owner, nsp) AS (
  SELECT 'pg_catalog.pg_namespace'::pg_catalog.regclass::oid, n.oid, n.nspowner, n.oid FROM pg_catalog.pg_namespace n
  UNION ALL SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::oid, c.oid, c.relowner, c.relnamespace FROM pg_catalog.pg_class c
  UNION ALL SELECT 'pg_catalog.pg_type'::pg_catalog.regclass::oid, t.oid, t.typowner, t.typnamespace FROM pg_catalog.pg_type t
  UNION ALL SELECT 'pg_catalog.pg_proc'::pg_catalog.regclass::oid, p.oid, p.proowner, p.pronamespace FROM pg_catalog.pg_proc p
  UNION ALL SELECT 'pg_catalog.pg_collation'::pg_catalog.regclass::oid, c.oid, c.collowner, c.collnamespace FROM pg_catalog.pg_collation c
  UNION ALL SELECT 'pg_catalog.pg_conversion'::pg_catalog.regclass::oid, c.oid, c.conowner, c.connamespace FROM pg_catalog.pg_conversion c
  UNION ALL SELECT 'pg_catalog.pg_operator'::pg_catalog.regclass::oid, o.oid, o.oprowner, o.oprnamespace FROM pg_catalog.pg_operator o
  UNION ALL SELECT 'pg_catalog.pg_opclass'::pg_catalog.regclass::oid, o.oid, o.opcowner, o.opcnamespace FROM pg_catalog.pg_opclass o
  UNION ALL SELECT 'pg_catalog.pg_opfamily'::pg_catalog.regclass::oid, o.oid, o.opfowner, o.opfnamespace FROM pg_catalog.pg_opfamily o
  UNION ALL SELECT 'pg_catalog.pg_ts_config'::pg_catalog.regclass::oid, c.oid, c.cfgowner, c.cfgnamespace FROM pg_catalog.pg_ts_config c
  UNION ALL SELECT 'pg_catalog.pg_ts_dict'::pg_catalog.regclass::oid, d.oid, d.dictowner, d.dictnamespace FROM pg_catalog.pg_ts_dict d
  UNION ALL SELECT 'pg_catalog.pg_statistic_ext'::pg_catalog.regclass::oid, s.oid, s.stxowner, s.stxnamespace FROM pg_catalog.pg_statistic_ext s
  UNION ALL SELECT 'pg_catalog.pg_extension'::pg_catalog.regclass::oid, e.oid, e.extowner, e.extnamespace FROM pg_catalog.pg_extension e
  UNION ALL SELECT 'pg_catalog.pg_publication'::pg_catalog.regclass::oid, p.oid, p.pubowner, 0::oid FROM pg_catalog.pg_publication p
  UNION ALL SELECT 'pg_catalog.pg_event_trigger'::pg_catalog.regclass::oid, e.oid, e.evtowner, 0::oid FROM pg_catalog.pg_event_trigger e
  UNION ALL SELECT 'pg_catalog.pg_foreign_data_wrapper'::pg_catalog.regclass::oid, w.oid, w.fdwowner, 0::oid FROM pg_catalog.pg_foreign_data_wrapper w
  UNION ALL SELECT 'pg_catalog.pg_foreign_server'::pg_catalog.regclass::oid, s.oid, s.srvowner, 0::oid FROM pg_catalog.pg_foreign_server s
  UNION ALL SELECT 'pg_catalog.pg_language'::pg_catalog.regclass::oid, l.oid, l.lanowner, 0::oid FROM pg_catalog.pg_language l
  UNION ALL SELECT 'pg_catalog.pg_largeobject_metadata'::pg_catalog.regclass::oid, l.oid, l.lomowner, 0::oid FROM pg_catalog.pg_largeobject_metadata l
  UNION ALL SELECT 'pg_catalog.pg_default_acl'::pg_catalog.regclass::oid, a.oid, a.defaclrole, a.defaclnamespace FROM pg_catalog.pg_default_acl a),
candidate(classid, objid) AS (
  SELECT o.classid, o.objid FROM owned o
  WHERE (o.objid>=16384 OR o.classid IN ('pg_catalog.pg_largeobject_metadata'::pg_catalog.regclass, 'pg_catalog.pg_default_acl'::pg_catalog.regclass))
   AND o.owner<>(SELECT oid FROM me) AND o.nsp NOT IN (SELECT oid FROM temp)
   AND NOT (o.classid='pg_catalog.pg_namespace'::pg_catalog.regclass AND o.objid=$1)
  UNION ALL
  SELECT 'pg_catalog.pg_cast'::pg_catalog.regclass::oid, c.oid FROM pg_catalog.pg_cast c WHERE c.oid>=16384
   AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_type t WHERE t.oid IN (c.castsource, c.casttarget) AND t.typowner=(SELECT oid FROM me))
  UNION ALL SELECT 'pg_catalog.pg_transform'::pg_catalog.regclass::oid, t.oid FROM pg_catalog.pg_transform t WHERE t.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_am'::pg_catalog.regclass::oid, a.oid FROM pg_catalog.pg_am a WHERE a.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_ts_parser'::pg_catalog.regclass::oid, p.oid FROM pg_catalog.pg_ts_parser p WHERE p.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_ts_template'::pg_catalog.regclass::oid, t.oid FROM pg_catalog.pg_ts_template t WHERE t.oid>=16384),
part_of(classid, objid, of_class, of_obj) AS (
  SELECT c.classid, c.objid, c.classid, c.objid FROM candidate c
  UNION
  SELECT u.classid, u.objid, d.refclassid, d.refobjid FROM part_of u
  JOIN pg_catalog.pg_depend d ON d.classid=u.of_class AND d.objid=u.of_obj AND d.deptype IN ('e','i','a'))
SELECT pg_catalog.pg_describe_object(c.classid, c.objid, 0) FROM candidate c
WHERE NOT EXISTS (SELECT 1 FROM part_of u JOIN owned o ON o.classid=u.of_class AND o.objid=u.of_obj
  WHERE u.classid=c.classid AND u.objid=c.objid AND o.owner=(SELECT oid FROM me))
ORDER BY 1 LIMIT 101"#;

/// Privileges granted to the scratch role on objects it does not own, in
/// this database or on shared objects (databases, tablespaces,
/// parameters), and policies of other roles' tables that name it.
/// `DROP OWNED` revokes or changes them, and with a grant option a replayed
/// migration could change other databases. Those on `public` ($1) are
/// restored with it; those on the members of an extension the scratch role
/// installed go with the extension. A policy on its own table goes with
/// the table.
const FOREIGN_GRANTS: &str = r#"WITH me(oid) AS (SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=session_user)
SELECT pg_catalog.pg_describe_object(s.classid, s.objid, s.objsubid) FROM pg_catalog.pg_shdepend s
WHERE s.refclassid='pg_catalog.pg_authid'::pg_catalog.regclass AND s.refobjid=(SELECT oid FROM me)
 AND (s.dbid=0 OR s.dbid=(SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()))
 AND (s.deptype='a' OR (s.deptype='r' AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_policy p JOIN pg_catalog.pg_class c ON c.oid=p.polrelid
   WHERE p.oid=s.objid AND c.relowner=(SELECT oid FROM me))))
 AND NOT (s.dbid<>0 AND s.classid='pg_catalog.pg_namespace'::pg_catalog.regclass AND s.objid=$1)
 AND NOT (s.dbid<>0 AND EXISTS (SELECT 1 FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_extension e ON e.oid=d.refobjid
   WHERE d.classid=s.classid AND d.objid=s.objid AND d.refclassid='pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype='e' AND e.extowner=(SELECT oid FROM me)))
ORDER BY 1 LIMIT 101"#;

/// What outlives a scratch database's objects and the scratch role could
/// change: its own role (settings, comment, memberships), the databases
/// and tablespaces it owns, with their settings and comments. Its password
/// is not readable; `SHARED_WRITES` sees a change to it.
const SHARED_STATE: &str = r#"SELECT pg_catalog.jsonb_build_object(
 'pg_authid', (SELECT pg_catalog.to_jsonb(r) - 'rolpassword' FROM pg_catalog.pg_roles r WHERE r.oid=me.oid),
 'pg_database', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(d) - 'datfrozenxid' - 'datminmxid' ORDER BY d.oid) FROM pg_catalog.pg_database d WHERE d.datdba=me.oid),
 'pg_tablespace', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(t) ORDER BY t.oid) FROM pg_catalog.pg_tablespace t WHERE t.spcowner=me.oid),
 'pg_db_role_setting', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(s) ORDER BY s.setdatabase, s.setrole) FROM pg_catalog.pg_db_role_setting s
   WHERE s.setrole=me.oid OR s.setdatabase IN (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datdba=me.oid)),
 'pg_shdescription', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) ORDER BY c.classoid, c.objoid) FROM pg_catalog.pg_shdescription c
   WHERE (c.classoid='pg_catalog.pg_authid'::pg_catalog.regclass AND c.objoid=me.oid)
   OR (c.classoid='pg_catalog.pg_database'::pg_catalog.regclass AND c.objoid IN (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datdba=me.oid))
   OR (c.classoid='pg_catalog.pg_tablespace'::pg_catalog.regclass AND c.objoid IN (SELECT t.oid FROM pg_catalog.pg_tablespace t WHERE t.spcowner=me.oid))),
 'pg_auth_members', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(m) ORDER BY m.roleid, m.member, m.grantor) FROM pg_catalog.pg_auth_members m WHERE me.oid IN (m.roleid, m.member, m.grantor))
)::text FROM pg_catalog.pg_roles me WHERE me.rolname=session_user"#;

/// Shared catalogs this transaction wrote that no migration may change in
/// scratch: a new password (`pg_authid`, which only superusers read),
/// memberships, parameter privileges, security labels, replication
/// origins and subscriptions. These counters need track_counts, which
/// `prepare_scratch` requires. `pg_database` is compared by content
/// instead: a GRANT on another database that grants nothing still
/// rewrites its row.
const SHARED_WRITES: &str = r#"SELECT c.relname::text FROM pg_catalog.pg_class c WHERE c.relisshared AND c.relkind='r'
 AND c.relname IN ('pg_authid','pg_auth_members','pg_parameter_acl','pg_shseclabel','pg_replication_origin','pg_subscription')
 AND pg_catalog.pg_stat_get_xact_tuples_inserted(c.oid)+pg_catalog.pg_stat_get_xact_tuples_updated(c.oid)+pg_catalog.pg_stat_get_xact_tuples_deleted(c.oid)>0
ORDER BY 1"#;

/// What a scratch session's replay set up, for cleanup.
#[derive(Default)]
pub(super) struct Replay {
    /// The marker this session creates, set just before it is committed.
    id: std::sync::OnceLock<String>,
    /// Whether scratch may bypass row-level security, for the emptiness
    /// check after cleanup.
    pub(super) bypass_rls: std::sync::OnceLock<bool>,
}

/// What prepare saw in `public`, which cleanup restores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PublicState {
    pub(super) oid: u32,
    owner: String,
    acl: Option<String>,
    comment: Option<String>,
}

/// A replay marker as scratch holds it. The OIDs show whether a replayed
/// migration dropped and recreated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Marker {
    schema_oid: u32,
    table_oid: u32,
    id: String,
    started_at: String,
    pub(super) public: PublicState,
}

/// Replay steps and cleanup run their statements under replay's fixed
/// limit, as migrations do (`Limits::default`).
fn limit() -> Duration {
    Limits::default().statement()
}

/// `DB_SCRATCH_NOT_EMPTY` as `prepare_scratch` reports it.
pub(super) fn not_empty() -> Diagnostic {
    Diagnostic::new("DB_SCRATCH_NOT_EMPTY","Scratch must be an empty disposable database owned by an unprivileged, non-member role, with no user objects, event triggers, extensions or default grants.")
}

fn cleanup_failed(cause: Diagnostic) -> Diagnostic {
    if cause.code == "DB_SCRATCH_CLEANUP" {
        return cause;
    }
    let mut d = Diagnostic::new(
        "DB_SCRATCH_CLEANUP",
        &format!(
            "Replay could not clean up the scratch database ({}: {}); what it committed there and NTNT's marker schema _ntnt_replay remain.",
            cause.code, cause.message
        ),
    );
    d.objects = cause.objects;
    d
}

impl Postgres {
    /// Take the replay lock, waiting up to `wait` for another replay on
    /// this scratch database, then fail with `DB_SCRATCH_BUSY`. As with the
    /// migrator lock, the statement and client deadlines sit above the wait.
    pub(super) async fn lock_scratch(&self, wait: Duration) -> Result<()> {
        self.batch(&format!(
            "SET lock_timeout = '{}s'; SET statement_timeout = '{}s'",
            wait.as_secs(),
            wait.as_secs() + 1
        ))
        .await?;
        let locked = bounded_for(wait + SERVER_LIMIT_MARGIN, self.client.query(LOCK, &[])).await;
        self.batch("SET lock_timeout = '2s'; SET statement_timeout = 5000")
            .await?;
        locked.map(|_| ()).map_err(|d| {
            if d.code == "DB_LOCK_CONFLICT" {
                Diagnostic::new(
                    "DB_SCRATCH_BUSY",
                    &format!("Another ntnt plan, status or verify is replaying the history on this scratch database; it still held the replay lock after {} s. Nothing in scratch was changed.", wait.as_secs()),
                )
            } else {
                d
            }
        })
    }

    /// The marker in scratch, if it has the shape NTNT gives it; None when
    /// there is none or `_ntnt_replay` holds something else.
    pub(super) async fn read_marker(&self) -> Result<Option<Marker>> {
        let shape = self.rows(MARKER_SHAPE, &[]).await?;
        let Some(shape) = shape.first() else {
            return Ok(None);
        };
        if shape.get::<_, Option<bool>>(2) != Some(true) {
            return Ok(None);
        }
        let rows = self.rows(MARKER_ROW, &[]).await?;
        let [row] = rows.as_slice() else {
            return Ok(None);
        };
        let (Some(id), Some(started_at), Some(oid), Some(owner)) = (
            row.get::<_, Option<String>>(0),
            row.get::<_, Option<String>>(1),
            row.get::<_, Option<u32>>(2),
            row.get::<_, Option<String>>(3),
        ) else {
            return Ok(None);
        };
        Ok(Some(Marker {
            schema_oid: shape.get(0),
            table_oid: shape.get(1),
            id,
            started_at,
            public: PublicState {
                oid,
                owner,
                acl: row.get(4),
                comment: row.get(5),
            },
        }))
    }

    async fn public_state(&self) -> Result<Option<PublicState>> {
        Ok(self
            .rows(PUBLIC_STATE, &[])
            .await?
            .first()
            .map(|r| PublicState {
                oid: r.get(0),
                owner: r.get(1),
                acl: r.get(2),
                comment: r.get(3),
            }))
    }

    /// Objects another role owns in scratch (`FOREIGN_OBJECTS`), by name.
    async fn foreign_objects(&self, public: u32) -> Result<Vec<String>> {
        Ok(self
            .rows(FOREIGN_OBJECTS, &[&public])
            .await?
            .iter()
            .map(|r| r.get::<_, Option<String>>(0).unwrap_or_default())
            .collect())
    }

    /// Refuse a scratch role that `DROP OWNED` would take privileges from
    /// (`FOREIGN_GRANTS`), and a server whose statistics cannot show the
    /// writes `SHARED_WRITES` refuses.
    pub(super) async fn check_cleanable(&self, public: u32) -> Result<()> {
        let grants: Vec<String> = self
            .rows(FOREIGN_GRANTS, &[&public])
            .await?
            .iter()
            .map(|r| r.get::<_, Option<String>>(0).unwrap_or_default())
            .collect();
        if !grants.is_empty() {
            return Err(Diagnostic::new(
                "DB_SCRATCH_UNSAFE",
                &format!("The scratch role holds privileges on {} object(s) it does not own (its objects field names them). Replay cleans scratch up with DROP OWNED, which would revoke them, so nothing was run.", grants.len()),
            )
            .with_objects(&grants));
        }
        let counted: bool = self
            .rows(
                "SELECT pg_catalog.current_setting('track_counts')::bool",
                &[],
            )
            .await?[0]
            .get(0);
        if !counted {
            return Err(Diagnostic::new(
                "DB_SCRATCH_UNSAFE",
                "Scratch runs with track_counts off. Replay reads each migration's write counters to refuse role changes, such as a new password, that would outlive the replay, so nothing was run.",
            ));
        }
        Ok(())
    }

    /// Open the transaction one replayed migration runs and commits in:
    /// the target's migration transaction (`begin_migration`), except that
    /// scratch's commits need not wait for the disk.
    pub async fn begin_replay_step(&self) -> Result<()> {
        self.batch(&format!(
            "BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE; SET LOCAL synchronous_commit=off; {MIGRATION_TRANSACTION}"
        ))
        .await
    }

    /// End a replay step: commit what `migration` (None before the first)
    /// did, once `execute_captured` refused what cleanup could not undo
    /// (`refuse_lasting_effects`) and replay inspected it.
    pub async fn finish_replay_step(&self, migration: Option<&Migration>) -> Result<()> {
        // RESET ALL after each migration restored synchronous_commit.
        bounded_for(
            limit() + SERVER_LIMIT_MARGIN,
            self.client
                .batch_execute("SET LOCAL synchronous_commit=off; COMMIT"),
        )
        .await
        .map_err(|mut d| {
            d.source = migration.map(|m| m.name.clone());
            d
        })
    }

    /// Right after `migration` ran on scratch, before it is inspected and
    /// commits: it must leave the session as the scratch role, NTNT's marker
    /// as it was, the role and databases as they were, and nothing owned by
    /// another role.
    pub(super) async fn refuse_lasting_effects(
        &self,
        migration: &Migration,
        scope: &ScratchScope,
    ) -> Result<()> {
        let attributed = |code: &str, message: &str, objects: Vec<String>| {
            let mut d = Diagnostic::new(code, message).with_objects(&objects);
            d.source = Some(migration.name.clone());
            d
        };
        let acting: Option<String> = self
            .rows(
                "SELECT CASE WHEN current_user<>session_user THEN current_user::text END",
                &[],
            )
            .await?[0]
            .get(0);
        if let Some(role) = acting {
            return Err(attributed(
                "DB_REPLAY_UNSUPPORTED",
                &format!("Replaying this migration left the session acting as role {role} (SET ROLE in a DO block or function), so later migrations and NTNT's own statements would run as that role."),
                vec![],
            ));
        }
        if self.read_marker().await?.as_ref() != Some(&scope.marker) {
            return Err(protected_schema(
                migration,
                "Replaying this migration changed NTNT's replay marker, the schema _ntnt_replay.",
                vec![format!("schema:{MARKER_SCHEMA}")],
            ));
        }
        let state: String = self.rows(SHARED_STATE, &[]).await?[0].get(0);
        let mut changed: Vec<String> = self
            .rows(SHARED_WRITES, &[])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        if state != scope.shared {
            let (before, after): (Value, Value) = (
                serde_json::from_str(&scope.shared).unwrap_or_default(),
                serde_json::from_str(&state).unwrap_or_default(),
            );
            if let (Some(before), Some(after)) = (before.as_object(), after.as_object()) {
                changed.extend(
                    after
                        .iter()
                        .filter(|(k, v)| before.get(*k) != Some(v))
                        .map(|(k, _)| k.clone()),
                );
            }
        }
        if !changed.is_empty() {
            changed.sort();
            changed.dedup();
            return Err(attributed(
                "DB_REPLAY_UNSUPPORTED",
                &format!("Replaying this migration changed state that outlives the scratch database's objects ({}): ALTER DATABASE or ALTER ROLE settings, a database-level GRANT, REVOKE or COMMENT, or a password, made in a DO block or function. Cleanup cannot undo it, and on the target it would change the deployment database or role. Make such changes outside the migration history.", changed.join(", ")),
                changed,
            ));
        }
        let foreign: Vec<String> = self
            .foreign_objects(scope.marker.public.oid)
            .await?
            .into_iter()
            .filter(|o| !scope.foreign.contains(o))
            .collect();
        if !foreign.is_empty() {
            return Err(attributed(
                "DB_REPLAY_UNSUPPORTED",
                &format!("Replaying this migration left {} object(s) owned by a role other than the scratch role (its objects field names them), which cleanup's DROP OWNED would not drop. Give objects no other owner in migrations.", foreign.len()),
                foreign,
            ));
        }
        Ok(())
    }

    /// Commit the marker before the first migration: `replay_id` is set
    /// first, so cleanup still finds this replay's marker if the commit's
    /// outcome is unknown.
    pub(super) async fn create_marker(&self, public: &PublicState) -> Result<()> {
        let id = format!("{:032x}", rand::random::<u128>());
        let id = self.replay.id.get_or_init(|| id);
        self.batch(CREATE_MARKER).await?;
        self.rows(
            "INSERT INTO _ntnt_replay.replay_v1 VALUES ($1, pg_catalog.now(), $2, $3, $4, $5)",
            &[id, &public.oid, &public.owner, &public.acl, &public.comment],
        )
        .await?;
        Ok(())
    }

    /// What prepare must record before the first migration: `public`, and
    /// what `refuse_lasting_effects` compares: the shared state and the
    /// objects of other roles scratch already holds.
    pub(super) async fn scratch_baseline(&self) -> Result<(PublicState, String, Vec<String>)> {
        let public = self.public_state().await?.ok_or_else(not_empty)?;
        let shared: String = self.rows(SHARED_STATE, &[]).await?[0].get(0);
        let foreign = self.foreign_objects(public.oid).await?;
        Ok((public, shared, foreign))
    }

    /// The marker this session committed, for its scope.
    pub(super) async fn own_marker(&self) -> Result<Marker> {
        match self.read_marker().await? {
            Some(marker) if Some(&marker.id) == self.replay.id.get() => Ok(marker),
            _ => Err(not_empty()),
        }
    }

    /// A replay that never finished left its marker: clean scratch up when
    /// nothing in it has another owner, inside the caller's transaction,
    /// and say so. Without the marker scratch is never cleaned.
    pub(super) async fn recover_leftovers(&self) -> Result<Option<Value>> {
        let Some(marker) = self.read_marker().await? else {
            return Ok(None);
        };
        let foreign = self.foreign_objects(marker.public.oid).await?;
        if !foreign.is_empty() {
            return Err(not_empty().with_objects(&foreign));
        }
        self.clean(&marker.public).await?;
        Ok(Some(
            json!({"replay_id":marker.id,"started_at":marker.started_at}),
        ))
    }

    /// Drop what the scratch role owns and restore `public`, inside the
    /// caller's transaction.
    async fn clean(&self, public: &PublicState) -> Result<()> {
        // DROP OWNED BY CURRENT_USER must name the scratch role itself, never
        // a role a migration switched to.
        self.batch(&format!(
            "RESET ROLE; SET LOCAL statement_timeout='{}s'; SET LOCAL lock_timeout='{}s';",
            limit().as_secs(),
            limit().as_secs()
        ))
        .await?;
        if !self.rows("SELECT current_user=session_user", &[]).await?[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_SCRATCH_CLEANUP",
                "The scratch session acts as another role, so cleanup did not run.",
            ));
        }
        self.check_cleanable(public.oid).await?;
        // A public the scratch role owns would be dropped with the rest:
        // hand it to pg_database_owner until it is restored below.
        if let Some(row) = self
            .rows("SELECT n.nspname::text FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles me ON me.rolname=session_user WHERE n.oid=$1 AND n.nspowner=me.oid", &[&public.oid])
            .await?
            .first()
        {
            self.batch(&format!(
                "ALTER SCHEMA {} OWNER TO pg_database_owner",
                quote(&row.get::<_, String>(0))
            ))
            .await?;
        }
        let casts: Vec<String> = self
            .rows(DROP_CASTS, &[])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        for cast in casts {
            self.batch(&cast).await?;
        }
        bounded_for(
            limit() + SERVER_LIMIT_MARGIN,
            self.client.batch_execute("DROP OWNED BY CURRENT_USER"),
        )
        .await?;
        self.restore_public(public).await
    }

    /// Give `public` back the name, owner, privileges and comment it had
    /// before the first migration, recreating it if a migration dropped it.
    async fn restore_public(&self, public: &PublicState) -> Result<()> {
        let found: Option<String> = self
            .rows(
                "SELECT n.nspname::text FROM pg_catalog.pg_namespace n WHERE n.oid=$1",
                &[&public.oid],
            )
            .await?
            .first()
            .map(|r| r.get(0));
        match found.as_deref() {
            Some("public") => {}
            Some(name) => {
                self.batch(&format!("ALTER SCHEMA {} RENAME TO public", quote(name)))
                    .await?
            }
            None => {
                self.batch(&format!(
                    "CREATE SCHEMA public AUTHORIZATION {}",
                    quote(&public.owner)
                ))
                .await?
            }
        }
        let now = self.public_state().await?.ok_or_else(not_empty)?;
        if now.owner != public.owner {
            self.batch(&format!(
                "ALTER SCHEMA public OWNER TO {}",
                quote(&public.owner)
            ))
            .await?;
        }
        let grants: Vec<String> = self
            .rows(
                &format!("{PUBLIC_GRANTS} {RESTORE_PUBLIC_GRANTS}"),
                &[&public.acl, &public.owner],
            )
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        for grant in grants {
            self.batch(&grant).await?;
        }
        if self.public_state().await?.and_then(|s| s.comment) != public.comment {
            let comment: String = self
                .rows(
                    "SELECT pg_catalog.format('COMMENT ON SCHEMA public IS %s', coalesce(pg_catalog.quote_literal($1::text), 'NULL'))",
                    &[&public.comment],
                )
                .await?[0]
                .get(0);
            self.batch(&comment).await?;
        }
        let now = self.public_state().await?.ok_or_else(not_empty)?;
        let same_grants: bool = self
            .rows(
                &format!("{PUBLIC_GRANTS} SELECT NOT EXISTS (SELECT * FROM want EXCEPT SELECT * FROM have) AND NOT EXISTS (SELECT * FROM have EXCEPT SELECT * FROM want)"),
                &[&public.acl, &public.owner],
            )
            .await?[0]
            .get(0);
        if now.owner != public.owner || now.comment != public.comment || !same_grants {
            return Err(Diagnostic::new(
                "DB_SCRATCH_CLEANUP",
                "The public schema's owner, privileges or comment could not be restored to what they were before replay.",
            ));
        }
        Ok(())
    }

    /// After replay, whatever its outcome: end any open transaction, then
    /// drop what this replay committed, restore `public` and check scratch
    /// is empty again, in one transaction, and release the replay lock.
    /// Nothing is dropped unless scratch holds this replay's own marker.
    /// A failure is `DB_SCRATCH_CLEANUP`; the marker then stays, so the
    /// next replay cleans up.
    pub async fn cleanup_scratch(&self) -> Result<()> {
        let result = async {
            self.rollback().await?;
            let Some(id) = self.replay.id.get() else {
                return Ok(());
            };
            self.batch("BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE; SET LOCAL search_path=pg_catalog;")
                .await?;
            let marker = match self.read_marker().await? {
                Some(marker) if &marker.id == id => marker,
                // The marker never committed: there is nothing to clean.
                None => return self.rollback().await,
                Some(_) => return Err(not_empty()),
            };
            self.clean(&marker.public).await?;
            self.check_empty(self.replay.bypass_rls.get().copied().unwrap_or(false))
                .await?;
            self.batch("COMMIT").await
        }
        .await;
        if result.is_err() {
            let _ = self.rollback().await;
        }
        let unlocked = self.batch(UNLOCK).await;
        result.map_err(cleanup_failed).and(unlocked)
    }
}
