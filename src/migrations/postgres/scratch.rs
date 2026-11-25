//! Scratch replay's lifecycle on PostgreSQL (#430). Each replayed migration
//! commits on scratch in a transaction set up like the target's, so scratch
//! is cleaned up afterwards, and only what NTNT can prove it created is
//! ever dropped:
//!
//! - A session advisory lock serializes replays on one scratch database,
//!   from the first look at its contents until cleanup ends. A migration
//!   that releases it, also for a transaction-level lock on the same key
//!   that its commit would release, is refused before it commits, and the
//!   lock is checked again after each commit.
//! - Before the marker, the scratch role may own nothing in scratch but
//!   `public`, since cleanup drops whatever it owns.
//! - Before the first migration a committed marker, the schema
//!   `_ntnt_replay` with one row, records the replay and the state of
//!   `public`, which the scratch role can change (as a member of
//!   `pg_database_owner`) but does not own.
//! - Cleanup drops everything the scratch role owns (`DROP OWNED`), restores
//!   `public` and runs the emptiness check again, in one transaction.
//! - The marker is bound to this database (its OID and the cluster's system
//!   identifier) and records every transaction of the replay that wrote the
//!   catalog or a row of the scratch role's tables, in the same transaction
//!   as the write.
//! - A marker that a replay which never finished left behind is cleaned
//!   the same way, but only when it was written in this database, scratch
//!   holds no `_ntnt` ledger, no other role owns anything in scratch, every
//!   catalog row there was last written by one of the marker's transactions,
//!   so was every row of the scratch role's tables, and no other session of
//!   the scratch role, which may still be that replay, is connected.
//! - The scratch database's privileges, connection limit and comment, and
//!   the settings of the database and the scratch role, are recorded in the
//!   marker and restored by cleanup, like `public`.
//! - Changes cleanup cannot undo (other databases, a password, memberships,
//!   objects given to another role) are refused before the migration that
//!   made them commits.
use super::super::catalog_sql::declared_owned;
use super::super::inventory::{Limits, Migration};
use super::super::{Diagnostic, Result};
use super::{
    bounded_for, protected_schema, quote, Postgres, ScratchScope, IO_TIMEOUT,
    MIGRATION_TRANSACTION, SERVER_LIMIT_MARGIN,
};
use serde_json::{json, Value};
use std::time::Duration;

/// The replay lock: a session advisory lock on the scratch database. The
/// target's migrator lock uses the same first key with 245.
const LOCK: &str = "SELECT pg_catalog.pg_advisory_lock(1853124212, 430)";
const UNLOCK: &str = "SELECT pg_catalog.pg_advisory_unlock(1853124212, 430)";
/// Whether this session still holds the replay lock: a migration can
/// release it (`pg_advisory_unlock_all()`).
const HOLDS_LOCK: &str = "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_locks l WHERE l.locktype='advisory' AND l.classid=1853124212 AND l.objid=430 AND l.objsubid=2 AND l.pid=pg_catalog.pg_backend_pid() AND l.granted)";
/// `pg_locks` shows a transaction-level lock on the same key the same way,
/// and a migration may take one in place of the replay lock; its commit then
/// releases it. In the open transaction, once `HOLDS_LOCK` holds, this takes
/// a transaction-level hold too, so that the key stays held throughout:
/// releasing one session-level hold (`SESSION_UNLOCK`) then succeeds only if
/// there is one, and `LOCK` takes it back at once.
const XACT_LOCK: &str = "SELECT pg_catalog.pg_advisory_xact_lock(1853124212, 430)";
const SESSION_UNLOCK: &str = "SELECT pg_catalog.pg_advisory_unlock(1853124212, 430)";
/// Whether another session of the scratch role is connected to scratch. A
/// replay whose migration released the replay lock may still be running.
const OTHER_SESSIONS: &str = "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity a WHERE a.datname=pg_catalog.current_database() AND a.usename=session_user AND a.backend_type='client backend' AND a.pid<>pg_catalog.pg_backend_pid())";

/// The marker's schema and table. Its one row names the replay and holds
/// the state `public` had before the first migration, which cleanup
/// restores, also after a replay that never finished.
pub(super) const MARKER_SCHEMA: &str = "_ntnt_replay";
const CREATE_MARKER: &str = "CREATE SCHEMA _ntnt_replay; CREATE TABLE _ntnt_replay.replay_v1 (id text, started_at timestamptz, system_identifier text, database_oid oid, public_oid oid, public_owner text, public_acl text, public_comment text, database_acl text, database_connlimit integer, database_comment text, settings text, xids xid8[]);";

/// The marker's shape: `_ntnt_replay` and `replay_v1` owned by this role,
/// a plain table whose reads run no code (no rules, triggers or row
/// security), with exactly these columns, and nothing else in the schema.
/// A table's row type depends on the table, not the schema.
const MARKER_SHAPE: &str = r#"SELECT n.oid, c.oid, n.nspowner=me.oid AND c.relowner=me.oid AND c.relkind='r' AND c.relpersistence='p'
 AND NOT c.relhasrules AND NOT c.relhastriggers AND NOT c.relrowsecurity
 AND (SELECT pg_catalog.count(*) FROM pg_catalog.pg_depend d WHERE d.refclassid='pg_catalog.pg_namespace'::pg_catalog.regclass AND d.refobjid=n.oid)=1
 AND (SELECT pg_catalog.array_agg(a.attname::text||':'||a.atttypid::pg_catalog.regtype::text ORDER BY a.attnum) FROM pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped)
  = ARRAY['id:text','started_at:timestamp with time zone','system_identifier:text','database_oid:oid','public_oid:oid','public_owner:text','public_acl:text','public_comment:text','database_acl:text','database_connlimit:integer','database_comment:text','settings:text','xids:xid8[]']
FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles me ON me.rolname=session_user
LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid AND c.relname='replay_v1'
WHERE n.nspname='_ntnt_replay'"#;
const MARKER_ROW: &str = r#"SELECT id, pg_catalog.to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'), public_oid, public_owner, public_acl, public_comment,
 system_identifier, database_oid, database_acl, database_connlimit, database_comment, settings,
 xids IS NOT NULL AND NOT EXISTS (SELECT FROM pg_catalog.unnest(xids) u(x) WHERE u.x IS NULL)
FROM ONLY _ntnt_replay.replay_v1 LIMIT 2"#;
/// The transactions the marker records, as text: those of prepare and of
/// each committed replay step that wrote the catalog (`record_xids`).
const MARKER_XIDS: &str = "SELECT coalesce(xids::text, '') FROM ONLY _ntnt_replay.replay_v1";

/// What a superuser installed for the declared extensions (`$2`, #430),
/// as a `WITH RECURSIVE` element: `declared(classid, objid)`, each declared
/// extension, the schema it is installed in and what it owns
/// (`DECLARED_OWNED`). Scratch holds them before the marker, as the target
/// does; `DROP OWNED` keeps them, since another role owns them, so recovery
/// neither counts them as another role's objects nor asks a replay to have
/// written them.
macro_rules! declared_objects {
    () => {
        concat!(
            declared_owned!("$2"),
            r#",
declared(classid, objid) AS (
  SELECT 'pg_catalog.pg_extension'::pg_catalog.regclass::oid, e.oid FROM pg_catalog.pg_extension e WHERE e.extname=ANY($2::text[])
  UNION SELECT 'pg_catalog.pg_namespace'::pg_catalog.regclass::oid, e.extnamespace FROM pg_catalog.pg_extension e WHERE e.extname=ANY($2::text[])
  UNION SELECT o.classid, o.objid FROM owned o)"#
        )
    };
}

/// The catalog rows of this database's objects that `initdb` did not create
/// (OIDs from 16384, and every large object and default privilege), with
/// the transaction that last wrote each row (`xmin`) and the object's
/// namespace (0 when it has none): each object's own row and the rows of
/// what is part of it (columns, indexes, sequences, enum labels,
/// constraints, triggers, policies, rules, defaults and comments). The rows
/// keep `xmin` when they are frozen or updated in place, as by VACUUM and
/// ANALYZE.
const CATALOG_ROWS: &str = r#"cat(classid, objid, nsp, x) AS (
  SELECT 'pg_catalog.pg_namespace'::pg_catalog.regclass::oid, n.oid, n.oid, n.xmin FROM pg_catalog.pg_namespace n WHERE n.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::oid, c.oid, c.relnamespace, c.xmin FROM pg_catalog.pg_class c WHERE c.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::oid, a.attrelid, c.relnamespace, a.xmin FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid WHERE a.attrelid>=16384
  UNION ALL SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::oid, i.indexrelid, c.relnamespace, i.xmin FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indexrelid WHERE i.indexrelid>=16384
  UNION ALL SELECT 'pg_catalog.pg_class'::pg_catalog.regclass::oid, q.seqrelid, c.relnamespace, q.xmin FROM pg_catalog.pg_sequence q JOIN pg_catalog.pg_class c ON c.oid=q.seqrelid WHERE q.seqrelid>=16384
  UNION ALL SELECT 'pg_catalog.pg_type'::pg_catalog.regclass::oid, t.oid, t.typnamespace, t.xmin FROM pg_catalog.pg_type t WHERE t.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_type'::pg_catalog.regclass::oid, e.enumtypid, t.typnamespace, e.xmin FROM pg_catalog.pg_enum e JOIN pg_catalog.pg_type t ON t.oid=e.enumtypid WHERE e.enumtypid>=16384
  UNION ALL SELECT 'pg_catalog.pg_proc'::pg_catalog.regclass::oid, p.oid, p.pronamespace, p.xmin FROM pg_catalog.pg_proc p WHERE p.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_collation'::pg_catalog.regclass::oid, c.oid, c.collnamespace, c.xmin FROM pg_catalog.pg_collation c WHERE c.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_conversion'::pg_catalog.regclass::oid, c.oid, c.connamespace, c.xmin FROM pg_catalog.pg_conversion c WHERE c.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_operator'::pg_catalog.regclass::oid, o.oid, o.oprnamespace, o.xmin FROM pg_catalog.pg_operator o WHERE o.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_opclass'::pg_catalog.regclass::oid, o.oid, o.opcnamespace, o.xmin FROM pg_catalog.pg_opclass o WHERE o.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_opfamily'::pg_catalog.regclass::oid, o.oid, o.opfnamespace, o.xmin FROM pg_catalog.pg_opfamily o WHERE o.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_ts_config'::pg_catalog.regclass::oid, c.oid, c.cfgnamespace, c.xmin FROM pg_catalog.pg_ts_config c WHERE c.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_ts_dict'::pg_catalog.regclass::oid, d.oid, d.dictnamespace, d.xmin FROM pg_catalog.pg_ts_dict d WHERE d.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_ts_parser'::pg_catalog.regclass::oid, p.oid, p.prsnamespace, p.xmin FROM pg_catalog.pg_ts_parser p WHERE p.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_ts_template'::pg_catalog.regclass::oid, t.oid, t.tmplnamespace, t.xmin FROM pg_catalog.pg_ts_template t WHERE t.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_statistic_ext'::pg_catalog.regclass::oid, s.oid, s.stxnamespace, s.xmin FROM pg_catalog.pg_statistic_ext s WHERE s.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_extension'::pg_catalog.regclass::oid, e.oid, e.extnamespace, e.xmin FROM pg_catalog.pg_extension e WHERE e.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_constraint'::pg_catalog.regclass::oid, k.oid, k.connamespace, k.xmin FROM pg_catalog.pg_constraint k WHERE k.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_trigger'::pg_catalog.regclass::oid, g.oid, c.relnamespace, g.xmin FROM pg_catalog.pg_trigger g JOIN pg_catalog.pg_class c ON c.oid=g.tgrelid WHERE g.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_policy'::pg_catalog.regclass::oid, p.oid, c.relnamespace, p.xmin FROM pg_catalog.pg_policy p JOIN pg_catalog.pg_class c ON c.oid=p.polrelid WHERE p.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_rewrite'::pg_catalog.regclass::oid, r.oid, c.relnamespace, r.xmin FROM pg_catalog.pg_rewrite r JOIN pg_catalog.pg_class c ON c.oid=r.ev_class WHERE r.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_attrdef'::pg_catalog.regclass::oid, d.oid, c.relnamespace, d.xmin FROM pg_catalog.pg_attrdef d JOIN pg_catalog.pg_class c ON c.oid=d.adrelid WHERE d.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_cast'::pg_catalog.regclass::oid, c.oid, 0::oid, c.xmin FROM pg_catalog.pg_cast c WHERE c.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_transform'::pg_catalog.regclass::oid, t.oid, 0::oid, t.xmin FROM pg_catalog.pg_transform t WHERE t.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_am'::pg_catalog.regclass::oid, a.oid, 0::oid, a.xmin FROM pg_catalog.pg_am a WHERE a.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_language'::pg_catalog.regclass::oid, l.oid, 0::oid, l.xmin FROM pg_catalog.pg_language l WHERE l.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_publication'::pg_catalog.regclass::oid, p.oid, 0::oid, p.xmin FROM pg_catalog.pg_publication p
  UNION ALL SELECT 'pg_catalog.pg_event_trigger'::pg_catalog.regclass::oid, e.oid, 0::oid, e.xmin FROM pg_catalog.pg_event_trigger e
  UNION ALL SELECT 'pg_catalog.pg_foreign_data_wrapper'::pg_catalog.regclass::oid, w.oid, 0::oid, w.xmin FROM pg_catalog.pg_foreign_data_wrapper w WHERE w.oid>=16384
  UNION ALL SELECT 'pg_catalog.pg_foreign_server'::pg_catalog.regclass::oid, s.oid, 0::oid, s.xmin FROM pg_catalog.pg_foreign_server s
  UNION ALL SELECT 'pg_catalog.pg_largeobject_metadata'::pg_catalog.regclass::oid, l.oid, 0::oid, l.xmin FROM pg_catalog.pg_largeobject_metadata l
  UNION ALL SELECT 'pg_catalog.pg_default_acl'::pg_catalog.regclass::oid, a.oid, a.defaclnamespace, a.xmin FROM pg_catalog.pg_default_acl a
  UNION ALL SELECT d.classoid, d.objoid, 0::oid, d.xmin FROM pg_catalog.pg_description d WHERE d.objoid>=16384)"#;

/// Add to the marker, in the transaction about to commit, this transaction
/// and every transaction of its tree (its own and its subtransactions', as
/// PL/pgSQL exception blocks start) that last wrote a row of `CATALOG_ROWS`
/// or of `tables`, the scratch role's tables it wrote (`WRITTEN_TABLES`):
/// rows that are visible and whose `xmin` is still in progress can only be
/// this transaction's. A child's transaction ID is always above its
/// parent's; $1 is one just assigned (`record_xids`), so no ID above it is
/// looked up. Returns the marker's transactions.
fn record_xids_sql(tables: &[String]) -> String {
    let data: String = tables
        .iter()
        .map(|t| format!(" UNION ALL SELECT s.xmin FROM ONLY {t} s"))
        .collect();
    format!(
        r#"WITH {CATALOG_ROWS},
written(x) AS (SELECT c.x FROM cat c{data}),
top AS (SELECT pg_catalog.pg_current_xact_id()::text::numeric AS x),
bound AS (SELECT t.x, ($1::text::numeric - t.x % 4294967296 + 4294967296) % 4294967296 AS upto FROM top t),
d AS (SELECT DISTINCT (w.x::text::numeric - t.x % 4294967296 + 4294967296) % 4294967296 AS d FROM written w, top t UNION SELECT 0),
mine AS (SELECT (t.x + d.d)::text::pg_catalog.xid8 AS x FROM d, bound t
  WHERE CASE WHEN d.d <= t.upto THEN pg_catalog.pg_xact_status((t.x + d.d)::text::pg_catalog.xid8) = 'in progress' ELSE false END)
UPDATE ONLY _ntnt_replay.replay_v1 r SET xids = r.xids || ARRAY(SELECT m.x FROM mine m WHERE m.x <> ALL (r.xids) ORDER BY m.x)
RETURNING coalesce(r.xids::text, '')"#
    )
}

/// The scratch role's tables this transaction wrote rows of, by
/// qualified name, for `record_xids_sql`. Not those with row-level security
/// forced on their owner: replay runs with `row_security` off, so reading
/// them would fail. Their rows written in a subtransaction stay unrecorded,
/// which can only make recovery refuse.
const WRITTEN_TABLES: &str = r#"SELECT pg_catalog.format('%I.%I', n.nspname, c.relname) FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_roles me ON me.oid=c.relowner AND me.rolname=session_user
WHERE c.relkind='r' AND NOT (c.relrowsecurity AND c.relforcerowsecurity) AND n.nspname<>'_ntnt_replay' AND n.nspname !~ '^pg_(toast_)?temp_[0-9]+$'
 AND pg_catalog.pg_stat_get_xact_tuples_inserted(c.oid)+pg_catalog.pg_stat_get_xact_tuples_updated(c.oid)>0
 AND pg_catalog.has_schema_privilege(n.oid, 'USAGE') AND pg_catalog.has_table_privilege(c.oid, 'SELECT')
ORDER BY c.oid"#;

/// The scratch role's tables, but the marker's and temporary ones, when
/// scratch holds a marker: what an unfinished replay may have left, by
/// qualified name, as `pg_describe_object` names it, whether this session
/// holds a lock on it that keeps other sessions from writing it, whether
/// row-level security is forced on its owner, whether the scratch role can
/// lock it (`prepare_scratch` takes SHARE, which needs its schema's USAGE and
/// UPDATE, DELETE or TRUNCATE on it), and whether it lacks SELECT on it: a
/// migration may revoke the owner's own privileges.
pub(super) const LEFTOVER_TABLES: &str = r#"SELECT pg_catalog.format('%I.%I', n.nspname, c.relname), pg_catalog.pg_describe_object('pg_catalog.pg_class'::pg_catalog.regclass, c.oid, 0),
 EXISTS (SELECT 1 FROM pg_catalog.pg_locks l WHERE l.locktype='relation' AND l.relation=c.oid AND l.pid=pg_catalog.pg_backend_pid() AND l.granted
  AND l.mode IN ('ShareLock','ShareRowExclusiveLock','ExclusiveLock','AccessExclusiveLock')),
 c.relrowsecurity AND c.relforcerowsecurity,
 pg_catalog.has_schema_privilege(n.oid, 'USAGE') AND pg_catalog.has_table_privilege(c.oid, 'UPDATE, DELETE, TRUNCATE'),
 NOT pg_catalog.has_table_privilege(c.oid, 'SELECT')
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_roles me ON me.oid=c.relowner AND me.rolname=session_user
WHERE c.relkind='r' AND n.nspname<>'_ntnt_replay' AND n.nspname !~ '^pg_(toast_)?temp_[0-9]+$'
 AND pg_catalog.to_regnamespace('_ntnt_replay') IS NOT NULL
ORDER BY c.oid"#;

/// Which of `tables` (by position, from 1) hold a row whose `xmin` is none
/// of the marker's transactions, compared as 32-bit IDs, as in
/// `unproven_sql`.
fn unproven_rows_sql(tables: &[String]) -> String {
    let checks: Vec<String> = tables
        .iter()
        .enumerate()
        .map(|(i, t)| {
            format!(
                "SELECT {} AS i WHERE EXISTS (SELECT 1 FROM ONLY {t} s WHERE NOT EXISTS (SELECT 1 FROM rec WHERE rec.x=s.xmin::text::bigint))",
                i + 1
            )
        })
        .collect();
    format!(
        "WITH rec(x) AS (SELECT (u.x::text::numeric % 4294967296)::bigint FROM ONLY _ntnt_replay.replay_v1 r CROSS JOIN LATERAL pg_catalog.unnest(r.xids) u(x))\n{} ORDER BY 1",
        checks.join("\nUNION ALL ")
    )
}

/// What recovery cannot prove the marker's replay wrote, by name: the rows
/// of `CATALOG_ROWS` whose `xmin` is none of the marker's transactions,
/// leaving out `public` ($1, which cleanup restores), the temporary
/// schemas of sessions and what a superuser installed for the declared
/// extensions (`declared_objects`, `$2`) before any replay, which no replay
/// transaction wrote. The comparison is of 32-bit transaction IDs, so
/// `TOO_OLD` refuses a marker whose transactions are too old for it.
fn unproven_sql() -> String {
    format!(
        r#"WITH RECURSIVE {CATALOG_ROWS},
{declared},
temp(oid) AS (SELECT n.oid FROM pg_catalog.pg_namespace n WHERE n.nspname ~ '^pg_(toast_)?temp_[0-9]+$'),
rec(x) AS (SELECT (u.x::text::numeric % 4294967296)::bigint FROM ONLY _ntnt_replay.replay_v1 r CROSS JOIN LATERAL pg_catalog.unnest(r.xids) u(x))
SELECT DISTINCT pg_catalog.pg_describe_object(c.classid, c.objid, 0) FROM cat c
WHERE c.nsp NOT IN (SELECT oid FROM temp)
 AND NOT (c.classid='pg_catalog.pg_namespace'::pg_catalog.regclass AND c.objid=$1)
 AND NOT EXISTS (SELECT 1 FROM declared w WHERE w.classid=c.classid AND w.objid=c.objid)
 AND NOT EXISTS (SELECT 1 FROM rec WHERE rec.x=c.x::text::bigint)
ORDER BY 1 LIMIT 101"#,
        declared = declared_objects!()
    )
}
/// Whether the marker's oldest transaction is more than 2^31 transactions
/// old, or it records none, so that `xmin` cannot tell its rows apart.
const TOO_OLD: &str = "SELECT coalesce((SELECT pg_catalog.pg_snapshot_xmax(pg_catalog.pg_current_snapshot())::text::numeric - pg_catalog.min(u.x::text::numeric) FROM ONLY _ntnt_replay.replay_v1 r CROSS JOIN LATERAL pg_catalog.unnest(r.xids) u(x)) > 2147483647, true)";

/// The scratch database's privileges, connection limit and comment, and the
/// settings (`pg_db_role_setting`) of the scratch database and of the
/// scratch role, which cleanup restores. A setting row is keyed by database
/// and role OID, 0 meaning all.
const DATABASE_STATE: &str = r#"SELECT d.datacl::text, d.datconnlimit, pg_catalog.shobj_description(d.oid, 'pg_database')::text,
 coalesce((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('database', s.setdatabase::pg_catalog.int8, 'role', s.setrole::pg_catalog.int8, 'config', s.setconfig) ORDER BY s.setdatabase, s.setrole)
   FROM pg_catalog.pg_db_role_setting s WHERE s.setdatabase=d.oid OR s.setrole=me.oid), '[]')::text,
 d.oid, me.oid, d.datname::text
FROM pg_catalog.pg_database d, pg_catalog.pg_roles me WHERE d.datname=pg_catalog.current_database() AND me.rolname=session_user"#;

/// Privileges the scratch database had ($1, as `datacl::text`) but not now,
/// or now but not then, as sets of (grantor, grantee, privilege, grant
/// option); NULL means the owner's default privileges.
const DATABASE_GRANTS: &str = r#"WITH want AS (SELECT a.grantor, a.grantee, a.privilege_type, a.is_grantable
  FROM pg_catalog.pg_database d CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce($1::text::pg_catalog.aclitem[], pg_catalog.acldefault('d', d.datdba))) a WHERE d.datname=pg_catalog.current_database()),
have AS (SELECT a.grantor, a.grantee, a.privilege_type, a.is_grantable FROM pg_catalog.pg_database d
  CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(d.datacl, pg_catalog.acldefault('d', d.datdba))) a WHERE d.datname=pg_catalog.current_database())"#;

/// Statements that give the scratch database its privileges back, as
/// `RESTORE_PUBLIC_GRANTS` does for `public`. Run by the owner, each grant
/// records the owner as grantor.
const RESTORE_DATABASE_GRANTS: &str = r#"SELECT s FROM (
  SELECT 1 AS o, 'REVOKE ALL ON DATABASE '||pg_catalog.quote_ident(pg_catalog.current_database())||' FROM '||pg_catalog.string_agg(DISTINCT CASE WHEN h.grantee=0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(h.grantee)) END, ', ')||' CASCADE' AS s FROM have h
  UNION ALL
  SELECT 2, 'GRANT '||pg_catalog.string_agg(w.privilege_type, ', ' ORDER BY w.privilege_type)||' ON DATABASE '||pg_catalog.quote_ident(pg_catalog.current_database())||' TO '||CASE WHEN w.grantee=0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantee)) END||CASE WHEN w.is_grantable THEN ' WITH GRANT OPTION' ELSE '' END
  FROM want w GROUP BY w.grantee, w.is_grantable) x
WHERE s IS NOT NULL AND (EXISTS (SELECT * FROM want EXCEPT SELECT * FROM have) OR EXISTS (SELECT * FROM have EXCEPT SELECT * FROM want))
ORDER BY o, s"#;

/// After `DATABASE_GRANTS`: the grants of the scratch database's privileges
/// before the first migration ($1) that a role other than its owner made,
/// with a grant option, when those privileges changed since. Cleanup gives
/// them back as the owner, and a grant records who made it, so it could not
/// restore these.
const DELEGATED_DATABASE_GRANTS: &str = r#"SELECT pg_catalog.format('%s on database %I granted to %s by %s', w.privilege_type, pg_catalog.current_database(),
  CASE WHEN w.grantee=0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantee)) END, pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantor)))
FROM want w WHERE w.grantor<>(SELECT d.datdba FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database())
 AND (EXISTS (SELECT * FROM want EXCEPT SELECT * FROM have) OR EXISTS (SELECT * FROM have EXCEPT SELECT * FROM want))
ORDER BY 1"#;

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

/// After `PUBLIC_GRANTS`, as `DELEGATED_DATABASE_GRANTS` for `public`
/// (owner $2).
const DELEGATED_PUBLIC_GRANTS: &str = r#"SELECT pg_catalog.format('%s on schema public granted to %s by %s', w.privilege_type,
  CASE WHEN w.grantee=0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantee)) END, pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantor)))
FROM want w WHERE w.grantor<>(SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=$2)
 AND (EXISTS (SELECT * FROM want EXCEPT SELECT * FROM have) OR EXISTS (SELECT * FROM have EXCEPT SELECT * FROM want))
ORDER BY 1"#;

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
/// it casts one of its types. `public` ($1, by OID, or the schema now
/// named `public`, which a migration may have dropped and created again for
/// `pg_database_owner`; only $1 when none is) is restored instead. Objects
/// `initdb` created have
/// OIDs below 16384. What a superuser installed for the declared extensions
/// (`declared_objects`, `$2`) is not listed: scratch holds it before the
/// marker, as the target does, and cleanup keeps it (#430).
const FOREIGN_OBJECTS: &str = concat!(
    "WITH RECURSIVE ",
    declared_objects!(),
    r#",
me(oid) AS (SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=session_user),
temp(oid) AS (SELECT n.oid FROM pg_catalog.pg_namespace n WHERE n.nspname ~ '^pg_(toast_)?temp_[0-9]+$' OR n.nspname IN ('pg_catalog','pg_toast','information_schema')),
objs(classid, objid, owner, nsp) AS (
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
  SELECT o.classid, o.objid FROM objs o
  WHERE (o.objid>=16384 OR o.classid IN ('pg_catalog.pg_largeobject_metadata'::pg_catalog.regclass, 'pg_catalog.pg_default_acl'::pg_catalog.regclass))
   AND o.owner<>(SELECT oid FROM me) AND o.nsp NOT IN (SELECT oid FROM temp)
   AND NOT (o.classid='pg_catalog.pg_namespace'::pg_catalog.regclass AND (o.objid=$1 OR o.objid IS NOT DISTINCT FROM pg_catalog.to_regnamespace('public')::oid))
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
WHERE NOT EXISTS (SELECT 1 FROM part_of u JOIN objs o ON o.classid=u.of_class AND o.objid=u.of_obj
  WHERE u.classid=c.classid AND u.objid=c.objid AND o.owner=(SELECT oid FROM me))
 AND NOT EXISTS (SELECT 1 FROM declared w WHERE w.classid=c.classid AND w.objid=c.objid)
ORDER BY 1 LIMIT 101"#
);

/// Privileges granted to the scratch role on objects it does not own, in
/// this database or on shared objects (databases, tablespaces,
/// parameters), and policies of other roles' tables that name it.
/// `DROP OWNED` revokes or changes them, and with a grant option a replayed
/// migration could change other databases. Those on `public` ($1) are
/// restored with it; those on the members of an extension the scratch role
/// installed go with the extension. A policy on its own table goes with
/// the table. The second column names the declared extension (`$2`) whose
/// object, or schema, the privilege is on (`declared_objects`, #430): a DBA
/// granted it, and the scratch role cannot grant it back.
const FOREIGN_GRANTS: &str = concat!(
    "WITH RECURSIVE ",
    declared_objects!(),
    r#",
me(oid) AS (SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=session_user)
SELECT pg_catalog.pg_describe_object(s.classid, s.objid, s.objsubid),
 CASE WHEN s.dbid<>0 AND EXISTS (SELECT 1 FROM declared w WHERE w.classid=s.classid AND w.objid=s.objid) THEN coalesce(
  (SELECT pg_catalog.min(e.extname::text) FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_extension e ON e.oid=d.refobjid
    WHERE d.classid=s.classid AND d.objid=s.objid AND d.refclassid='pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype='e' AND e.extname=ANY($2::text[])),
  (SELECT pg_catalog.min(e.extname::text) FROM pg_catalog.pg_extension e WHERE s.classid='pg_catalog.pg_namespace'::pg_catalog.regclass AND e.extnamespace=s.objid AND e.extname=ANY($2::text[])),
  '') END
FROM pg_catalog.pg_shdepend s
WHERE s.refclassid='pg_catalog.pg_authid'::pg_catalog.regclass AND s.refobjid=(SELECT oid FROM me)
 AND (s.dbid=0 OR s.dbid=(SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()))
 AND (s.deptype='a' OR (s.deptype='r' AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_policy p JOIN pg_catalog.pg_class c ON c.oid=p.polrelid
   WHERE p.oid=s.objid AND c.relowner=(SELECT oid FROM me))))
 AND NOT (s.dbid<>0 AND s.classid='pg_catalog.pg_namespace'::pg_catalog.regclass AND (s.objid=$1 OR s.objid IS NOT DISTINCT FROM pg_catalog.to_regnamespace('public')::oid))
 AND NOT (s.dbid<>0 AND EXISTS (SELECT 1 FROM pg_catalog.pg_depend d JOIN pg_catalog.pg_extension e ON e.oid=d.refobjid
   WHERE d.classid=s.classid AND d.objid=s.objid AND d.refclassid='pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype='e' AND e.extowner=(SELECT oid FROM me)))
ORDER BY 1 LIMIT 101"#
);

/// What the scratch role owns in scratch, other than `public` ($1), by
/// name: what cleanup's `DROP OWNED` would drop. Before the marker this must
/// be nothing, so that cleanup only drops what replay created; the
/// emptiness check does not see large objects or publications, say.
const OWNED_OBJECTS: &str = r#"SELECT pg_catalog.pg_describe_object(s.classid, s.objid, s.objsubid) FROM pg_catalog.pg_shdepend s
WHERE s.deptype='o' AND s.refclassid='pg_catalog.pg_authid'::pg_catalog.regclass
 AND s.refobjid=(SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=session_user)
 AND s.dbid=(SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database())
 AND NOT (s.classid='pg_catalog.pg_namespace'::pg_catalog.regclass AND s.objid=$1)
ORDER BY 1 LIMIT 101"#;

/// What outlives a scratch database's objects, the scratch role could
/// change and cleanup does not restore: its own role (attributes, comment,
/// memberships), the databases and tablespaces it owns, with the settings
/// and comments of the other databases. The scratch database's privileges,
/// connection limit, comment and settings, and the scratch role's settings,
/// are restored instead (`DATABASE_STATE`). Its password is not readable;
/// `SHARED_WRITES` sees a change to it.
const SHARED_STATE: &str = r#"SELECT pg_catalog.jsonb_build_object(
 'pg_authid', (SELECT pg_catalog.to_jsonb(r) - 'rolpassword' - 'rolconfig' FROM pg_catalog.pg_roles r WHERE r.oid=me.oid),
 'pg_database', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(d) - 'datfrozenxid' - 'datminmxid' - CASE WHEN d.datname=pg_catalog.current_database() THEN ARRAY['datacl','datconnlimit'] ELSE '{}'::text[] END ORDER BY d.oid) FROM pg_catalog.pg_database d WHERE d.datdba=me.oid),
 'pg_tablespace', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(t) ORDER BY t.oid) FROM pg_catalog.pg_tablespace t WHERE t.spcowner=me.oid),
 'pg_db_role_setting', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(s) ORDER BY s.setdatabase, s.setrole) FROM pg_catalog.pg_db_role_setting s
   WHERE s.setrole<>me.oid AND s.setdatabase IN (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datdba=me.oid AND d.datname<>pg_catalog.current_database())),
 'pg_shdescription', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) ORDER BY c.classoid, c.objoid) FROM pg_catalog.pg_shdescription c
   WHERE (c.classoid='pg_catalog.pg_authid'::pg_catalog.regclass AND c.objoid=me.oid)
   OR (c.classoid='pg_catalog.pg_database'::pg_catalog.regclass AND c.objoid IN (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datdba=me.oid AND d.datname<>pg_catalog.current_database()))
   OR (c.classoid='pg_catalog.pg_tablespace'::pg_catalog.regclass AND c.objoid IN (SELECT t.oid FROM pg_catalog.pg_tablespace t WHERE t.spcowner=me.oid))),
 'pg_auth_members', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(m) ORDER BY m.roleid, m.member, m.grantor) FROM pg_catalog.pg_auth_members m WHERE me.oid IN (m.roleid, m.member, m.grantor))
)::text FROM pg_catalog.pg_roles me WHERE me.rolname=session_user"#;

/// Shared catalogs this transaction wrote that no migration may change in
/// scratch: a new password (`pg_authid`, which only superusers read),
/// memberships, parameter privileges, security labels, replication
/// origins and subscriptions. These counters need track_counts, which
/// `prepare_scratch` requires. They count every write of the transaction,
/// also one a subtransaction rolled back (a password changed in an
/// exception block that then raised): the scratch role cannot read
/// `pg_authid` to compare its content. `pg_database` is compared by content
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
    /// The transactions the marker records, as each replayed migration must
    /// leave them (`MARKER_XIDS`).
    xids: std::sync::Mutex<String>,
    /// The TLS connector this session connected with, to cancel a statement
    /// that replay's deadline interrupted.
    pub(super) tls: Option<super::super::postgres_tls::MakeRustls>,
}

impl Replay {
    pub(super) fn new(tls: Option<super::super::postgres_tls::MakeRustls>) -> Self {
        Self {
            tls,
            ..Self::default()
        }
    }
}

/// What prepare saw in `public`, which cleanup restores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PublicState {
    pub(super) oid: u32,
    owner: String,
    acl: Option<String>,
    comment: Option<String>,
}

/// What prepare saw of the scratch database and the settings of it and of
/// the scratch role (`DATABASE_STATE`), which cleanup restores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DatabaseState {
    acl: Option<String>,
    connlimit: i32,
    comment: Option<String>,
    /// `pg_db_role_setting` rows as JSON.
    settings: String,
}

/// A replay marker as scratch holds it. The OIDs show whether a replayed
/// migration dropped and recreated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Marker {
    schema_oid: u32,
    table_oid: u32,
    id: String,
    started_at: String,
    /// The cluster (`pg_control_system().system_identifier`) and database
    /// the marker was written in.
    system_identifier: String,
    database_oid: u32,
    pub(super) public: PublicState,
    database: DatabaseState,
}

/// Settings whose values are lists of quoted elements: `pg_dump` sets them
/// element by element, as an `ALTER ... SET` of one string would quote the
/// whole list as one element.
const LIST_QUOTE_SETTINGS: [&str; 6] = [
    "local_preload_libraries",
    "search_path",
    "session_preload_libraries",
    "shared_preload_libraries",
    "temp_tablespaces",
    "unix_socket_directories",
];

/// A setting's stored value as the `SET ... TO` arguments that store it
/// again unchanged, as `pg_dump` writes them. Each is an escape string
/// literal, which keeps a backslash whatever `standard_conforming_strings`
/// says (NTNT's sessions set it on).
fn setting_value(name: &str, value: &str) -> String {
    let literal = |v: &str| format!("E'{}'", v.replace('\\', "\\\\").replace('\'', "''"));
    if !LIST_QUOTE_SETTINGS.contains(&name.to_ascii_lowercase().as_str()) {
        return literal(value);
    }
    let mut elements = vec![];
    let mut chars = value.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut element = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                    } else {
                        break;
                    }
                }
                element.push(c);
            }
            while chars.peek().is_some_and(|c| *c != ',') {
                chars.next();
            }
        } else {
            while let Some(c) = chars.peek().copied() {
                if c == ',' {
                    break;
                }
                element.push(c);
                chars.next();
            }
            element = element.trim_end().to_owned();
        }
        elements.push(literal(&element));
        if chars.next().is_none() {
            break;
        }
    }
    if value.trim().is_empty() {
        return "''".into();
    }
    elements.join(", ")
}

/// `pg_db_role_setting` rows (`DATABASE_STATE`) by database and role, each
/// with its settings by name.
type Settings = std::collections::BTreeMap<(u32, u32), std::collections::BTreeMap<String, String>>;

/// None when `json` is not such a snapshot: the marker's is then not NTNT's,
/// and cleanup restores nothing from it.
fn parse_settings(json: &str) -> Option<Settings> {
    let mut settings = Settings::new();
    let rows: Value = serde_json::from_str(json).ok()?;
    for row in rows.as_array()? {
        let oid = |key: &str| u32::try_from(row.get(key)?.as_u64()?).ok();
        let entries = settings
            .entry((oid("database")?, oid("role")?))
            .or_default();
        for item in row.get("config")?.as_array()? {
            let (name, value) = item.as_str()?.split_once('=')?;
            entries.insert(name.to_owned(), value.to_owned());
        }
    }
    settings.retain(|_, entries| !entries.is_empty());
    Some(settings)
}

/// Settings a session loads code for as it starts: a library that does not
/// load refuses the connection, an administrator's too.
const PRELOAD_SETTINGS: [&str; 2] = ["local_preload_libraries", "session_preload_libraries"];

/// `PRELOAD_SETTINGS` values by `pg_db_role_setting` row (database, role)
/// and name.
type Preload = Vec<((u32, u32), String, String)>;

/// The `PRELOAD_SETTINGS` that apply to the scratch role's sessions in the
/// scratch database (`database`, `role`), by row and name; None for a
/// malformed snapshot.
fn preload_settings(json: &str, database: u32, role: u32) -> Option<Preload> {
    let settings = parse_settings(json)?;
    let mut found = vec![];
    for key in [(database, 0), (0, role), (database, role)] {
        for (name, value) in settings.get(&key).into_iter().flatten() {
            if PRELOAD_SETTINGS.contains(&name.to_ascii_lowercase().as_str()) {
                found.push((key, name.to_ascii_lowercase(), value.clone()));
            }
        }
    }
    found.sort();
    Some(found)
}

/// Replay steps and cleanup run their statements under replay's fixed
/// limit, as migrations do (`Limits::default`).
fn limit() -> Duration {
    Limits::default().statement()
}

/// Cleanup's statements wait at most 2 seconds for each lock and run at most
/// 5 seconds, NTNT's limits for catalog exchanges; `DROP OWNED`, whose work
/// grows with the history, has `DROP_OWNED_LIMIT` (#430).
const CLEANUP_LIMITS: &str = "SET LOCAL statement_timeout=5000; SET LOCAL lock_timeout='2s';";
/// Dropping what an 800-migration history commits takes well under a second.
const DROP_OWNED_LIMIT: Duration = Duration::from_secs(10);
/// A lock wait `DROP OWNED` gives up on is tried again only when no session
/// holds a lock on scratch's objects any more: autovacuum gives way to a
/// waiting lock after `deadlock_timeout`.
const DROP_OWNED_ATTEMPTS: usize = 3;
/// Cleanup's transaction stops after this, leaving the cancel, rollback and
/// unlock exchanges their 5 seconds each within its 30 seconds
/// (`report::SCRATCH_CLEANUP_RESERVE`).
const CLEANUP_WORK: Duration = Duration::from_secs(15);
/// Sessions other than this one that hold a lock on a relation the scratch
/// role owns, or on another object, in the scratch database: whatever
/// `DROP OWNED` could wait for. The role is NULL for a background process
/// (autovacuum). Statistics are read fresh, not from the transaction's
/// snapshot of them.
const LOCK_HOLDERS: &str = r#"SELECT DISTINCT l.pid, a.usename::text, a.usename=session_user
FROM pg_catalog.pg_locks l LEFT JOIN pg_catalog.pg_stat_activity a ON a.pid=l.pid
WHERE l.granted AND l.pid<>pg_catalog.pg_backend_pid()
  AND l.database=(SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database())
  AND (l.locktype='object' OR (l.locktype IN ('relation','page','tuple') AND l.relation IN
       (SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_roles me ON me.oid=c.relowner WHERE me.rolname=session_user)))
ORDER BY l.pid"#;

/// `DB_SCRATCH_CLEANUP` for a `DROP OWNED` that could not take its locks
/// within the lock limit: it names the sessions that hold locks on what the
/// replay committed, and says the next replay recovers once they are gone.
fn cleanup_blocked(holders: &[(i32, Option<String>, Option<bool>)], attempts: usize) -> Diagnostic {
    let named: Vec<String> = holders
        .iter()
        .map(|(pid, role, mine)| match (role, mine) {
            (Some(role), Some(true)) => format!("pid {pid} (the scratch role {role})"),
            (Some(role), _) => format!("pid {pid} (role {role})"),
            (None, _) => format!("pid {pid} (a background process, such as autovacuum)"),
        })
        .collect();
    let who = if named.is_empty() {
        "the session that held it had already released it".to_owned()
    } else {
        format!(
            "{} other session(s) hold locks on what the replay committed there: {}",
            named.len(),
            named.join(", ")
        )
    };
    let mut d = Diagnostic::new(
        "DB_SCRATCH_CLEANUP",
        &format!(
            "Replay could not clean up the scratch database: DROP OWNED BY CURRENT_USER waited 2 seconds for a lock, the limit for each of cleanup's locks, {attempts} time(s) (SQLSTATE 55P03), and {who}. Nothing was dropped; what the replay committed there and NTNT's marker schema _ntnt_replay remain."
        ),
    );
    let own = if holders.iter().any(|(_, _, mine)| *mine == Some(true)) {
        " A session of the scratch role must not stay connected to scratch: while one is, the next replay refuses with DB_SCRATCH_BUSY and changes nothing."
    } else {
        ""
    };
    d.next_action = format!("Let the sessions named in the message finish, or end them as the scratch database's administrator (pg_terminate_backend), then rerun plan, status or verify: the next one finds NTNT's marker and cleans up what this replay committed before it replays.{own} The target was not changed.");
    d
}

/// `DB_SCRATCH_NOT_EMPTY` as `prepare_scratch` reports it.
pub(super) fn not_empty() -> Diagnostic {
    Diagnostic::new("DB_SCRATCH_NOT_EMPTY","Scratch must be an empty disposable database owned by an unprivileged, non-member role, with no user objects, event triggers, extensions or default grants.")
}

/// A marker whose leftovers recovery cannot prove are its replay's: nothing
/// is dropped.
fn unproven(marker: &Marker, why: &str, objects: &[String]) -> Diagnostic {
    let mut d = Diagnostic::new(
        "DB_SCRATCH_NOT_EMPTY",
        &format!("Scratch holds NTNT's marker of replay {}, started {}, but {why} So NTNT cannot prove that what the scratch role owns there is only what that replay committed, and nothing was dropped.", marker.id, marker.started_at),
    )
    .with_objects(objects);
    d.next_action = "If this database really is only this configuration's scratch database, drop and recreate it (or drop what it holds) as its administrator, then rerun. If another project or application uses it, point [databases.<name>.scratch] at a dedicated empty database instead and leave this one alone.".into();
    d
}

/// Releasing the replay lock failed after cleanup finished (#464 review):
/// cleanup's outcome stands, and closing the session released the lock.
fn unlock_failed(mut cause: Diagnostic) -> Diagnostic {
    cause.message = format!(
        "Scratch cleanup finished (scratch.cleanup says how), but releasing NTNT's replay lock afterwards failed ({}: {}), so NTNT closed the scratch session, which releases the lock.",
        cause.code, cause.message
    );
    cause
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

    async fn holds_lock(&self) -> Result<bool> {
        Ok(self.rows(HOLDS_LOCK, &[]).await?[0].get(0))
    }

    /// Whether this session holds the replay lock at session level, in the
    /// open transaction (`XACT_LOCK`): a transaction-level lock on the same
    /// key would end with the commit.
    async fn holds_session_lock(&self) -> Result<bool> {
        if !self.holds_lock().await? {
            return Ok(false);
        }
        self.batch(XACT_LOCK).await?;
        let session: bool = self.rows(SESSION_UNLOCK, &[]).await?[0].get(0);
        if session {
            self.batch(LOCK).await?;
        }
        Ok(session)
    }

    /// Cancel the statement this session is running, if any: replay's
    /// deadline dropped it before cleanup.
    pub async fn cancel_running(&self) {
        let token = self.client.cancel_token();
        let _ = match &self.replay.tls {
            Some(tls) => tokio::time::timeout(IO_TIMEOUT, token.cancel_query(tls.clone())).await,
            None => {
                tokio::time::timeout(IO_TIMEOUT, token.cancel_query(tokio_postgres::NoTls)).await
            }
        };
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
        let (
            Some(id),
            Some(started_at),
            Some(oid),
            Some(owner),
            Some(system_identifier),
            Some(database_oid),
            Some(connlimit),
            Some(settings),
        ) = (
            row.get::<_, Option<String>>(0),
            row.get::<_, Option<String>>(1),
            row.get::<_, Option<u32>>(2),
            row.get::<_, Option<String>>(3),
            row.get::<_, Option<String>>(6),
            row.get::<_, Option<u32>>(7),
            row.get::<_, Option<i32>>(9),
            row.get::<_, Option<String>>(11),
        )
        else {
            return Ok(None);
        };
        // Recovery proves provenance with the transactions the marker
        // records and restores the settings it records: a list with a NULL
        // in it, or settings that are not a snapshot, make it not NTNT's.
        if row.get::<_, Option<bool>>(12) != Some(true) || parse_settings(&settings).is_none() {
            return Ok(None);
        }
        Ok(Some(Marker {
            schema_oid: shape.get(0),
            table_oid: shape.get(1),
            id,
            started_at,
            system_identifier,
            database_oid,
            public: PublicState {
                oid,
                owner,
                acl: row.get(4),
                comment: row.get(5),
            },
            database: DatabaseState {
                acl: row.get(8),
                connlimit,
                comment: row.get(10),
                settings,
            },
        }))
    }

    async fn marker_xids(&self) -> Result<Option<String>> {
        Ok(self.rows(MARKER_XIDS, &[]).await?.first().map(|r| r.get(0)))
    }

    /// Record in the marker the transactions of the one about to commit
    /// that wrote the catalog (`record_xids_sql`), inside it.
    pub(super) async fn record_xids(&self) -> Result<()> {
        // A subtransaction's ID is above every ID assigned before it.
        self.batch("SAVEPOINT ntnt_replay_xids").await?;
        let bound: Option<String> = self
            .rows(
                "UPDATE ONLY _ntnt_replay.replay_v1 SET xids = xids RETURNING xmin::text",
                &[],
            )
            .await?
            .first()
            .map(|r| r.get(0));
        self.batch("RELEASE SAVEPOINT ntnt_replay_xids").await?;
        let bound = bound.ok_or_else(not_empty)?;
        let tables: Vec<String> = self
            .rows(WRITTEN_TABLES, &[])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        let xids: String = self.rows(&record_xids_sql(&tables), &[&bound]).await?[0].get(0);
        *self.replay.xids.lock().unwrap_or_else(|e| e.into_inner()) = xids;
        Ok(())
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

    async fn database_state(&self) -> Result<DatabaseState> {
        let rows = self.rows(DATABASE_STATE, &[]).await?;
        let row = rows.first().ok_or_else(not_empty)?;
        Ok(DatabaseState {
            acl: row.get(0),
            connlimit: row.get(1),
            comment: row.get(2),
            settings: row.get(3),
        })
    }

    /// Objects another role owns in scratch (`FOREIGN_OBJECTS`), by name.
    async fn foreign_objects(&self, public: u32) -> Result<Vec<String>> {
        let declared: &[String] = &self.extensions;
        Ok(self
            .rows(FOREIGN_OBJECTS, &[&public, &declared])
            .await?
            .iter()
            .map(|r| r.get::<_, Option<String>>(0).unwrap_or_default())
            .collect())
    }

    /// Refuse a scratch role that `DROP OWNED` would take privileges from
    /// (`FOREIGN_GRANTS`), and a server whose statistics cannot show the
    /// writes `SHARED_WRITES` refuses. A privilege a DBA granted it on what a
    /// declared extension installed (#430), such as USAGE on postgres_fdw's
    /// foreign-data wrapper, is refused by name: cleanup cannot restore it,
    /// since only the extension's owner, a superuser, can grant it again,
    /// and NTNT never connects to scratch as one. Granted to PUBLIC in
    /// scratch instead, it outlives `DROP OWNED`.
    pub(super) async fn check_cleanable(&self, public: u32) -> Result<()> {
        let declared: &[String] = &self.extensions;
        let rows: Vec<(String, Option<String>)> = self
            .rows(FOREIGN_GRANTS, &[&public, &declared])
            .await?
            .iter()
            .map(|r| {
                (
                    r.get::<_, Option<String>>(0).unwrap_or_default(),
                    r.get::<_, Option<String>>(1),
                )
            })
            .collect();
        let on_declared: Vec<String> = rows
            .iter()
            .filter(|(_, extension)| extension.is_some())
            .map(|(object, _)| object.clone())
            .collect();
        if !on_declared.is_empty() {
            let mut extensions: Vec<&str> = rows
                .iter()
                .filter_map(|(_, extension)| extension.as_deref())
                .filter(|extension| !extension.is_empty())
                .collect();
            extensions.sort_unstable();
            extensions.dedup();
            if extensions.is_empty() {
                extensions.push("declared");
            }
            let mut d = Diagnostic::new(
                "DB_SCRATCH_UNSAFE",
                &format!("The scratch role was granted privileges on {} object(s) that the {} extension(s) installed (its objects field names them). Replay cleans scratch up with DROP OWNED, which revokes every privilege granted to the scratch role, and the scratch role cannot grant these back: only their owner, the superuser who installed the extension, can. So nothing was run.", on_declared.len(), extensions.join(", ")),
            )
            .with_objects(&on_declared);
            d.next_action = "As a superuser in the scratch database, revoke those privileges from the scratch role and grant them to PUBLIC instead, for example REVOKE USAGE ON FOREIGN DATA WRAPPER postgres_fdw FROM <scratch role>; GRANT USAGE ON FOREIGN DATA WRAPPER postgres_fdw TO PUBLIC. DROP OWNED keeps what PUBLIC holds, scratch is a database only replay uses, and the privileges on what declared extensions install are not compared with the target's, where the migration role keeps its own grant. Then rerun. See docs/DATABASE_MIGRATIONS.md, Extensions.".into();
            return Err(d);
        }
        let grants: Vec<String> = rows.into_iter().map(|(object, _)| object).collect();
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

    /// End a replay step: record its transactions in the marker
    /// (`record_xids`), then commit what `migration` (None before the first)
    /// did, once `execute_captured` refused what cleanup could not undo
    /// (`refuse_lasting_effects`) and replay inspected it.
    pub async fn finish_replay_step(&self, migration: Option<&Migration>) -> Result<()> {
        let attributed = |mut d: Diagnostic| {
            d.source = migration.map(|m| m.name.clone());
            d
        };
        if self.scratch.get().is_some() {
            self.record_xids().await.map_err(attributed)?;
        }
        // RESET ALL after each migration restored synchronous_commit.
        bounded_for(
            limit() + SERVER_LIMIT_MARGIN,
            self.client
                .batch_execute("SET LOCAL synchronous_commit=off; COMMIT"),
        )
        .await
        .map_err(attributed)?;
        // `refuse_lasting_effects` refused a migration that released the
        // replay lock; check again now that the commit released every
        // transaction-level lock, before replay goes on.
        if self.scratch.get().is_some() && !self.holds_lock().await? {
            return Err(attributed(Diagnostic::new(
                "DB_REPLAY_UNSUPPORTED",
                "Replaying this migration released NTNT's replay lock on scratch as it committed, so another plan, status or verify could replay there at the same time. On the target it would release NTNT's migrator lock the same way. Release only the advisory locks the migration takes itself. Cleanup takes the replay lock again before it drops what the replay committed.",
            )));
        }
        Ok(())
    }

    /// Right after `migration` ran on scratch, before it is inspected and
    /// commits: it must leave the session as the scratch role and holding
    /// the replay lock, NTNT's marker as it was, the role and databases as
    /// they were (but for what cleanup restores), the scratch role able to
    /// connect to scratch again, and nothing owned by another role.
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
        if !self.holds_session_lock().await? {
            return Err(attributed(
                "DB_REPLAY_UNSUPPORTED",
                "Replaying this migration released NTNT's replay lock on scratch (pg_advisory_unlock_all() or pg_advisory_unlock(1853124212, 430)), also when it then took a transaction-level lock on the same key (pg_advisory_xact_lock(1853124212, 430)), which its commit would release, so another plan, status or verify could replay there at the same time. On the target it would release NTNT's migrator lock the same way. Release only the advisory locks the migration takes itself.",
                vec![],
            ));
        }
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
        let xids = self
            .replay
            .xids
            .lock()
            .map(|x| x.clone())
            .unwrap_or_default();
        if self.read_marker().await?.as_ref() != Some(&scope.marker)
            || self.marker_xids().await? != Some(xids)
        {
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
                &format!("Replaying this migration changed state that outlives the scratch database's objects and that cleanup does not restore ({}): a password, a membership, a role attribute or comment, or the settings, privileges or comment of another database, made in a DO block or function. PostgreSQL counts such a write even when a subtransaction rolled it back (an exception block that raised), since the scratch role cannot read pg_authid to compare. Cleanup cannot undo it, and on the target it would change the deployment role or another database. Make such changes outside the migration history.", changed.join(", ")),
                changed,
            ));
        }
        // Cleanup restores the scratch database's connection limit, but a
        // replay stopped before cleanup would leave a limit of 0, which
        // refuses every connection of the scratch role: no later plan could
        // connect to recover scratch.
        let limit: i32 = self
            .rows(
                "SELECT d.datconnlimit FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()",
                &[],
            )
            .await?[0]
            .get(0);
        if limit == 0 {
            return Err(attributed(
                "DB_REPLAY_UNSUPPORTED",
                "Replaying this migration set the scratch database's connection limit to 0 (ALTER DATABASE ... CONNECTION LIMIT 0, in a DO block or function). Cleanup would restore it, but a replay stopped before cleanup would leave scratch refusing every connection of the scratch role, so no later plan, status or verify could connect to clean it up; on the target it would refuse the application's connections. Make such changes outside the migration history.",
                vec![],
            ));
        }
        // The same for the scratch database's privileges: a migration may
        // revoke CONNECT from the scratch role, or from PUBLIC when the role
        // connects through PUBLIC's grant. The session acts as the scratch
        // role here (checked above), which is the role a new connection logs
        // in as. Cleanup restores the privileges in its one transaction, on
        // the connection it already holds, so restoring them cannot lock it
        // out.
        let row = self
            .rows(
                &format!(
                    "{DATABASE_GRANTS} SELECT pg_catalog.has_database_privilege(current_user, pg_catalog.current_database(), 'CONNECT'), ARRAY(SELECT DISTINCT CASE WHEN w.grantee=0 THEN 'PUBLIC' WHEN w.grantee=(SELECT r.oid FROM pg_catalog.pg_roles r WHERE r.rolname=current_user) THEN 'the scratch role '||pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantee)) ELSE 'role '||pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(w.grantee)) END FROM want w WHERE w.privilege_type='CONNECT' AND NOT EXISTS (SELECT FROM have h WHERE h.grantee=w.grantee AND h.privilege_type='CONNECT') ORDER BY 1)"
                ),
                &[&scope.marker.database.acl],
            )
            .await?;
        let (connect, revoked): (bool, Vec<String>) = (row[0].get(0), row[0].get(1));
        if !connect {
            let from = if revoked.is_empty() {
                String::new()
            } else {
                format!(" (it revoked CONNECT from {})", revoked.join(", "))
            };
            return Err(attributed(
                "DB_REPLAY_UNSUPPORTED",
                &format!("Replaying this migration left the scratch role without the CONNECT privilege on the scratch database{from}: REVOKE CONNECT ON DATABASE in a DO block or function, from the role itself or from PUBLIC when the role connects through PUBLIC's grant. Cleanup would restore the privilege, but a replay stopped before cleanup would leave the scratch role unable to connect to scratch, so no later plan, status or verify could clean it up; on the target it would lock the deployment role out of its database. Make such changes outside the migration history."),
                vec![],
            ));
        }
        // Settings that load libraries as a session starts: one that does not
        // load refuses every connection to scratch, the same way.
        let rows = self.rows(DATABASE_STATE, &[]).await?;
        let row = rows.first().ok_or_else(not_empty)?;
        let (database, role): (u32, u32) = (row.get(4), row.get(5));
        let preload = preload_settings(&row.get::<_, String>(3), database, role);
        if preload.is_none()
            || preload != preload_settings(&scope.marker.database.settings, database, role)
        {
            return Err(attributed(
                "DB_REPLAY_UNSUPPORTED",
                "Replaying this migration changed local_preload_libraries or session_preload_libraries for the scratch database or the scratch role (ALTER DATABASE or ALTER ROLE ... SET, in a DO block or function). Every new session there loads those libraries as it starts, and a library that does not load refuses the connection, also an administrator's. Cleanup would restore the setting, but a replay stopped before cleanup would leave scratch refusing every connection of the scratch role, so no later plan, status or verify could clean it up; on the target it would refuse the application's connections. Make such changes outside the migration history.",
                vec![],
            ));
        }
        // Cleanup gives the scratch database and `public` their privileges
        // back as their owner. A grant another role made with a grant option
        // cannot be made again that way, so cleanup could never restore a
        // change of privileges that held one: refuse it before it commits.
        let mut delegated: Vec<String> = self
            .rows(
                &format!("{DATABASE_GRANTS} {DELEGATED_DATABASE_GRANTS}"),
                &[&scope.marker.database.acl],
            )
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        delegated.extend(
            self.rows(
                &format!("{PUBLIC_GRANTS} {DELEGATED_PUBLIC_GRANTS}"),
                &[&scope.marker.public.acl, &scope.marker.public.owner],
            )
            .await?
            .iter()
            .map(|r| r.get::<_, String>(0)),
        );
        if !delegated.is_empty() {
            let mut d = attributed(
                "DB_REPLAY_UNSUPPORTED",
                &format!("Replaying this migration changed the privileges of the scratch database or of the schema public, and before replay they held {} grant(s) that a role other than the owner made with a grant option (its objects field names them). Cleanup restores those privileges as the owner, so each grant it makes records the owner as its grantor: it could not restore these, and scratch would stay changed.", delegated.len()),
                delegated,
            );
            d.next_action = "As the scratch database's administrator, grant those privileges again as the owner of the scratch database or of public (revoke them from the grantees named, then grant them as the owner), so that cleanup can restore them, then rerun. Scratch is a database only replay uses; the target is not affected.".into();
            return Err(d);
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
    /// outcome is unknown. It records `public`, the scratch database's
    /// state, and the cluster and database it was written in.
    pub(super) async fn create_marker(
        &self,
        public: &PublicState,
        cluster: &str,
        database_oid: u32,
    ) -> Result<()> {
        let database = self.database_state().await?;
        let id = format!("{:032x}", rand::random::<u128>());
        let id = self.replay.id.get_or_init(|| id);
        self.batch(CREATE_MARKER).await?;
        self.rows(
            "INSERT INTO _ntnt_replay.replay_v1 VALUES ($1, pg_catalog.now(), $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, '{}')",
            &[
                id,
                &cluster,
                &database_oid,
                &public.oid,
                &public.owner,
                &public.acl,
                &public.comment,
                &database.acl,
                &database.connlimit,
                &database.comment,
                &database.settings,
            ],
        )
        .await?;
        Ok(())
    }

    /// What prepare must record before the first migration: `public`, and
    /// what `refuse_lasting_effects` compares: the shared state and the
    /// objects of other roles scratch already holds. Anything the scratch
    /// role already owns but `public` is refused (`OWNED_OBJECTS`).
    pub(super) async fn scratch_baseline(&self) -> Result<(PublicState, String, Vec<String>)> {
        let public = self.public_state().await?.ok_or_else(not_empty)?;
        let owned: Vec<String> = self
            .rows(OWNED_OBJECTS, &[&public.oid])
            .await?
            .iter()
            .map(|r| r.get::<_, Option<String>>(0).unwrap_or_default())
            .collect();
        if !owned.is_empty() {
            return Err(not_empty().with_objects(&owned));
        }
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

    /// A replay that never finished left its marker: clean scratch up, inside
    /// the caller's transaction, and say so, but only when it can prove that
    /// everything `DROP OWNED` would drop is what that replay committed: the
    /// marker was written in this database of this cluster (`cluster`,
    /// `database_oid`), scratch holds no `_ntnt` ledger, nothing in it has
    /// another owner, every catalog row there was last written by one of the
    /// marker's transactions (`unproven_sql`), and no other session of the
    /// scratch role is connected. Without the marker scratch is never cleaned.
    pub(super) async fn recover_leftovers(
        &self,
        cluster: &str,
        database_oid: u32,
    ) -> Result<Option<Value>> {
        let Some(marker) = self.read_marker().await? else {
            return Ok(None);
        };
        // The lock no longer says whether that replay still runs: one of its
        // migrations may have released it. Its session would still be here.
        if self.rows(OTHER_SESSIONS, &[]).await?[0].get::<_, bool>(0) {
            return Err(Diagnostic::new(
                "DB_SCRATCH_BUSY",
                &format!("Scratch holds NTNT's marker of replay {}, started {}, and another session of the scratch role is still connected to scratch, so that replay may still be running. Nothing in scratch was changed.", marker.id, marker.started_at),
            ));
        }
        // A replay never commits NTNT's ledger in scratch: this database is,
        // or was, a deployment target.
        let ledger: bool = self
            .rows(
                "SELECT pg_catalog.to_regnamespace('_ntnt') IS NOT NULL",
                &[],
            )
            .await?[0]
            .get(0);
        if ledger {
            return Err(unproven(
                &marker,
                "it also holds the schema _ntnt, NTNT's migration ledger, which a replay never commits there: this database is, or was, a deployment target.",
                &["schema:_ntnt".into()],
            ));
        }
        if marker.system_identifier != cluster || marker.database_oid != database_oid {
            return Err(unproven(
                &marker,
                &format!("the marker was written in database OID {} of cluster {}, and this is database OID {database_oid} of cluster {cluster}: it was copied here (a dump and restore, a template or a physical copy).", marker.database_oid, marker.system_identifier),
                &[],
            ));
        }
        let foreign = self.foreign_objects(marker.public.oid).await?;
        if !foreign.is_empty() {
            return Err(not_empty().with_objects(&foreign));
        }
        if self.rows(TOO_OLD, &[]).await?[0].get::<_, bool>(0) {
            return Err(unproven(
                &marker,
                "its transactions are too old, or it records none, to tell its catalog rows from others.",
                &[],
            ));
        }
        let declared: &[String] = &self.extensions;
        let unproven_rows: Vec<String> = self
            .rows(&unproven_sql(), &[&marker.public.oid, &declared])
            .await?
            .iter()
            .map(|r| r.get::<_, Option<String>>(0).unwrap_or_default())
            .collect();
        if !unproven_rows.is_empty() {
            return Err(unproven(
                &marker,
                &format!("{} object(s) there (its objects field names them) were created or last changed by a transaction the marker does not record, outside that replay.", unproven_rows.len()),
                &unproven_rows,
            ));
        }
        self.check_leftover_rows(&marker).await?;
        self.clean(&marker).await?;
        Ok(Some(
            json!({"replay_id":marker.id,"started_at":marker.started_at}),
        ))
    }

    /// Catalog rows do not show who wrote a table's rows: another session may
    /// have written into a table the unfinished replay committed. Every row
    /// of the scratch role's tables must have been written by one of the
    /// marker's transactions, which replay records (`record_xids`).
    /// `prepare_scratch` locked them before this transaction's snapshot, so
    /// every row committed before is seen here and no session can write them
    /// until it ends. Row-level security forced on their owner is lifted for
    /// the read, and SELECT a migration revoked from the owner granted back,
    /// inside this transaction, which drops them.
    async fn check_leftover_rows(&self, marker: &Marker) -> Result<()> {
        let rows = self.rows(LEFTOVER_TABLES, &[]).await?;
        let tables: Vec<(String, String, bool, bool)> = rows
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect();
        let unreadable: Vec<String> = rows
            .iter()
            .filter(|r| r.get::<_, bool>(5))
            .map(|r| r.get(0))
            .collect();
        if tables.is_empty() {
            return Ok(());
        }
        let unlocked: Vec<String> = tables
            .iter()
            .filter(|t| !t.2)
            .map(|t| t.1.clone())
            .collect();
        if !unlocked.is_empty() {
            return Err(unproven(
                marker,
                &format!("recovery could not lock {} table(s) there (its objects field names them) against writes: the scratch role holds neither UPDATE, DELETE nor TRUNCATE on them, or no USAGE on their schema, or they appeared while recovery started.", unlocked.len()),
                &unlocked,
            ));
        }
        for name in &unreadable {
            self.batch(&format!("GRANT SELECT ON TABLE {name} TO CURRENT_USER"))
                .await?;
        }
        for (name, _, _, forced) in &tables {
            if *forced {
                self.batch(&format!(
                    "ALTER TABLE ONLY {name} NO FORCE ROW LEVEL SECURITY"
                ))
                .await?;
            }
        }
        let names: Vec<String> = tables.iter().map(|t| t.0.clone()).collect();
        let written: Vec<String> = self
            .rows(&unproven_rows_sql(&names), &[])
            .await?
            .iter()
            .filter_map(|r| {
                let i: i32 = r.get(0);
                tables
                    .get(usize::try_from(i).ok()?.checked_sub(1)?)
                    .map(|t| t.1.clone())
            })
            .collect();
        if !written.is_empty() {
            return Err(unproven(
                marker,
                &format!("{} table(s) there (its objects field names them) hold rows that a transaction the marker does not record wrote, outside that replay: another session wrote into what that replay committed.", written.len()),
                &written,
            ));
        }
        Ok(())
    }

    /// Drop what the scratch role owns and restore `public` and the scratch
    /// database's state, inside the caller's transaction, holding the replay
    /// lock.
    async fn clean(&self, marker: &Marker) -> Result<()> {
        let public = &marker.public;
        // DROP OWNED BY CURRENT_USER must name the scratch role itself, never
        // a role a migration switched to.
        self.batch(&format!("RESET ROLE; {CLEANUP_LIMITS}")).await?;
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
        // Another replay may run once this session lost the replay lock.
        if !self.holds_lock().await? {
            return Err(Diagnostic::new(
                "DB_SCRATCH_CLEANUP",
                "The scratch session no longer holds NTNT's replay lock, so another plan, status or verify may be replaying on scratch, and cleanup dropped nothing.",
            ));
        }
        self.drop_owned().await?;
        self.restore_public(public).await?;
        self.restore_database(&marker.database).await
    }

    /// `DROP OWNED BY CURRENT_USER` in a savepoint, under `DROP_OWNED_LIMIT`
    /// and the 2-second lock limit. A lock wait it gives up on fails with
    /// `cleanup_blocked`, naming the sessions that hold locks on scratch's
    /// objects; it is tried again, up to `DROP_OWNED_ATTEMPTS` times, only
    /// when none does any more (autovacuum gave way).
    async fn drop_owned(&self) -> Result<()> {
        for attempt in 1..=DROP_OWNED_ATTEMPTS {
            self.batch(&format!(
                "SAVEPOINT ntnt_drop_owned; SET LOCAL statement_timeout='{}s';",
                DROP_OWNED_LIMIT.as_secs()
            ))
            .await?;
            let dropped = bounded_for(
                DROP_OWNED_LIMIT + SERVER_LIMIT_MARGIN,
                self.client.batch_execute("DROP OWNED BY CURRENT_USER"),
            )
            .await;
            match dropped {
                Ok(()) => {
                    return self
                        .batch(&format!(
                            "RELEASE SAVEPOINT ntnt_drop_owned; {CLEANUP_LIMITS}"
                        ))
                        .await
                }
                Err(d) if d.code == "DB_LOCK_CONFLICT" => {
                    self.batch("ROLLBACK TO SAVEPOINT ntnt_drop_owned; RELEASE SAVEPOINT ntnt_drop_owned; SELECT pg_catalog.pg_stat_clear_snapshot();")
                        .await?;
                    let holders: Vec<(i32, Option<String>, Option<bool>)> = self
                        .rows(LOCK_HOLDERS, &[])
                        .await?
                        .iter()
                        .map(|r| (r.get(0), r.get(1), r.get(2)))
                        .collect();
                    if attempt == DROP_OWNED_ATTEMPTS || holders.iter().any(|h| h.1.is_some()) {
                        return Err(cleanup_blocked(&holders, attempt));
                    }
                }
                Err(d) => return Err(d),
            }
        }
        unreachable!("the last attempt returns")
    }

    /// Give `public` back the name, owner, privileges and comment it had
    /// before the first migration, recreating it if a migration dropped it.
    /// A `public` a migration created again in its place (for
    /// `pg_database_owner`) takes its place; one created beside a renamed
    /// original is dropped, if nothing is left in it.
    async fn restore_public(&self, public: &PublicState) -> Result<()> {
        let found: Option<String> = self
            .rows(
                "SELECT n.nspname::text FROM pg_catalog.pg_namespace n WHERE n.oid=$1",
                &[&public.oid],
            )
            .await?
            .first()
            .map(|r| r.get(0));
        let other = self.public_state().await?.filter(|s| s.oid != public.oid);
        match found.as_deref() {
            Some("public") => {}
            Some(name) => {
                if other.is_some() {
                    self.batch("DROP SCHEMA public RESTRICT").await?;
                }
                self.batch(&format!("ALTER SCHEMA {} RENAME TO public", quote(name)))
                    .await?
            }
            None if other.is_some() => {}
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

    /// Give the scratch database back the privileges (compared as sets, as
    /// `public`'s), connection limit and comment it had before the first
    /// migration, and the database and the scratch role their settings.
    async fn restore_database(&self, before: &DatabaseState) -> Result<()> {
        let rows = self.rows(DATABASE_STATE, &[]).await?;
        let row = rows.first().ok_or_else(not_empty)?;
        let (database, role, name): (u32, u32, String) = (row.get(4), row.get(5), row.get(6));
        let now = self.database_state().await?;
        // Both settings snapshots are read before anything is changed.
        let (Some(want), Some(have)) = (
            parse_settings(&before.settings),
            parse_settings(&now.settings),
        ) else {
            return Err(Diagnostic::new(
                "DB_SCRATCH_CLEANUP",
                "The settings NTNT's marker recorded for the scratch database and role, or those scratch holds now, are not a settings snapshot, so cleanup restored none of the scratch database's privileges, connection limit, comment or settings.",
            ));
        };
        let grants: Vec<String> = self
            .rows(
                &format!("{DATABASE_GRANTS} {RESTORE_DATABASE_GRANTS}"),
                &[&before.acl],
            )
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        for grant in grants {
            self.batch(&grant).await?;
        }
        if now.connlimit != before.connlimit {
            self.batch(&format!(
                "ALTER DATABASE {} CONNECTION LIMIT {}",
                quote(&name),
                before.connlimit
            ))
            .await?;
        }
        if now.comment != before.comment {
            let comment: String = self
                .rows(
                    "SELECT pg_catalog.format('COMMENT ON DATABASE %I IS %s', pg_catalog.current_database(), coalesce(pg_catalog.quote_literal($1::text), 'NULL'))",
                    &[&before.comment],
                )
                .await?[0]
                .get(0);
            self.batch(&comment).await?;
        }
        let empty = std::collections::BTreeMap::new();
        for key in want
            .keys()
            .chain(have.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let (w, h) = (
                want.get(key).unwrap_or(&empty),
                have.get(key).unwrap_or(&empty),
            );
            if w == h {
                continue;
            }
            let target = match *key {
                (d, 0) if d == database => format!("DATABASE {}", quote(&name)),
                (0, r) if r == role => "ROLE CURRENT_USER".to_owned(),
                (d, r) if r == role => {
                    let other: Option<String> = self
                        .rows(
                            "SELECT d.datname::text FROM pg_catalog.pg_database d WHERE d.oid=$1",
                            &[&d],
                        )
                        .await?
                        .first()
                        .map(|r| r.get(0));
                    let Some(other) = other else { continue };
                    format!("ROLE CURRENT_USER IN DATABASE {}", quote(&other))
                }
                _ => {
                    return Err(Diagnostic::new(
                        "DB_SCRATCH_CLEANUP",
                        "Settings of another role in the scratch database changed during replay, and the scratch role cannot restore them.",
                    ))
                }
            };
            for setting in w
                .keys()
                .chain(h.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let quoted = setting.split('.').map(quote).collect::<Vec<_>>().join(".");
                match w.get(setting) {
                    Some(value) if h.get(setting) != Some(value) => {
                        self.batch(&format!(
                            "ALTER {target} SET {quoted} TO {}",
                            setting_value(setting, value)
                        ))
                        .await?
                    }
                    Some(_) => {}
                    None => {
                        self.batch(&format!("ALTER {target} RESET {quoted}"))
                            .await?
                    }
                }
            }
        }
        let after = self.database_state().await?;
        let same_grants: bool = self
            .rows(
                &format!("{DATABASE_GRANTS} SELECT NOT EXISTS (SELECT * FROM want EXCEPT SELECT * FROM have) AND NOT EXISTS (SELECT * FROM have EXCEPT SELECT * FROM want)"),
                &[&before.acl],
            )
            .await?[0]
            .get(0);
        if !same_grants
            || after.connlimit != before.connlimit
            || after.comment != before.comment
            || parse_settings(&after.settings).as_ref() != Some(&want)
        {
            return Err(Diagnostic::new(
                "DB_SCRATCH_CLEANUP",
                "The scratch database's privileges, connection limit, comment or settings, or the scratch role's settings, could not be restored to what they were before replay.",
            ));
        }
        Ok(())
    }

    /// After replay, whatever its outcome: end any open transaction, then
    /// drop what this replay committed, restore `public` and the scratch
    /// database, and check scratch is empty again, in one transaction, and
    /// release the replay lock. Nothing is dropped unless scratch holds this
    /// replay's own marker and this session the replay lock, which it takes
    /// again if a migration released it. A failure is `DB_SCRATCH_CLEANUP`;
    /// the marker then stays, so the next replay cleans up. Ok(false) when
    /// the replay committed nothing, so there was nothing to clean. The
    /// transaction stops after `CLEANUP_WORK`, its statement cancelled.
    /// Releasing the lock comes after cleanup's outcome, and its failure
    /// comes back beside it, never instead of it: the session is then
    /// closed, which releases the lock, and what cleanup committed stays.
    pub async fn cleanup_scratch(&self) -> (Result<bool>, Option<Diagnostic>) {
        let work = async {
            self.rollback().await?;
            let Some(id) = self.replay.id.get() else {
                return Ok(false);
            };
            if !self.holds_lock().await?
                && !self
                    .rows(
                        "SELECT pg_catalog.pg_try_advisory_lock(1853124212, 430)",
                        &[],
                    )
                    .await?[0]
                    .get::<_, bool>(0)
            {
                return Err(Diagnostic::new(
                    "DB_SCRATCH_CLEANUP",
                    "A replayed migration released NTNT's replay lock and another session now holds it, so cleanup dropped nothing.",
                ));
            }
            self.batch("BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE; SET LOCAL search_path=pg_catalog;")
                .await?;
            let marker = match self.read_marker().await? {
                Some(marker) if &marker.id == id => marker,
                // The marker never committed: there is nothing to clean.
                None if self.scratch.get().is_none() => {
                    self.rollback().await?;
                    return Ok(false);
                }
                None => {
                    return Err(Diagnostic::new(
                        "DB_SCRATCH_CLEANUP",
                        "NTNT's replay marker, which this replay committed, is gone from scratch (another session dropped or changed the schema _ntnt_replay), so cleanup dropped nothing; what the replay committed there remains.",
                    ))
                }
                Some(_) => return Err(not_empty()),
            };
            self.clean(&marker).await?;
            self.check_empty(self.replay.bypass_rls.get().copied().unwrap_or(false), None)
                .await?;
            self.batch("COMMIT").await.map(|()| true)
        };
        let result = match tokio::time::timeout(CLEANUP_WORK, work).await {
            Ok(result) => result,
            Err(_) => {
                self.cancel_running().await;
                Err(Diagnostic::new(
                    "DB_SCRATCH_CLEANUP",
                    &format!("Replay could not clean up the scratch database within {} seconds, so NTNT cancelled cleanup; what the replay committed there and NTNT's marker schema _ntnt_replay remain.", CLEANUP_WORK.as_secs()),
                ))
            }
        };
        if result.is_err() {
            let _ = self.rollback().await;
        }
        let unlocked = self.batch(UNLOCK).await;
        if unlocked.is_err() {
            // Whether this session still holds the lock is unknown.
            self.task.abort();
        }
        match result {
            Ok(cleaned) => (Ok(cleaned), unlocked.err().map(unlock_failed)),
            Err(cause) => (Err(cleanup_failed(cause)), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cleanup_blocked, parse_settings, preload_settings, setting_value};
    use crate::migrations::Diagnostic;

    /// #457 review: a restored setting keeps its backslashes and
    /// apostrophes whatever standard_conforming_strings says.
    #[test]
    fn restored_setting_values_are_escape_literals() {
        assert_eq!(setting_value("application_name", r"a\b'c"), r"E'a\\b''c'");
        assert_eq!(
            setting_value("search_path", r#""x\y", public"#),
            r"E'x\\y', E'public'"
        );
        assert_eq!(setting_value("search_path", ""), "''");
    }

    /// #457 review: a settings snapshot that is not one is refused, not
    /// taken for no settings; an empty one is valid.
    #[test]
    fn a_malformed_settings_snapshot_is_none() {
        assert_eq!(parse_settings("[]"), Some(Default::default()));
        let valid =
            parse_settings(r#"[{"database": 5, "role": 0, "config": ["work_mem=5MB"]}]"#).unwrap();
        assert_eq!(valid[&(5, 0)]["work_mem"], "5MB");
        for bad in [
            "",
            "not json",
            "{}",
            "[1]",
            r#"[{"role": 0, "config": []}]"#,
            r#"[{"database": 5, "role": -1, "config": []}]"#,
            r#"[{"database": 5, "role": 4294967296, "config": []}]"#,
            r#"[{"database": 5, "role": 0}]"#,
            r#"[{"database": 5, "role": 0, "config": ["work_mem"]}]"#,
            r#"[{"database": 5, "role": 0, "config": [7]}]"#,
        ] {
            assert_eq!(parse_settings(bad), None, "{bad}");
        }
    }

    /// #458 review: the settings that load libraries as a session starts,
    /// for the scratch role's sessions in scratch only.
    #[test]
    fn preload_settings_are_those_of_scratch_sessions() {
        let json = r#"[{"database": 5, "role": 0, "config": ["Local_Preload_Libraries=a", "work_mem=1MB"]},
            {"database": 0, "role": 9, "config": ["session_preload_libraries=b"]},
            {"database": 6, "role": 9, "config": ["local_preload_libraries=c"]}]"#;
        assert_eq!(
            preload_settings(json, 5, 9).unwrap(),
            vec![
                (
                    (0, 9),
                    "session_preload_libraries".to_owned(),
                    "b".to_owned()
                ),
                ((5, 0), "local_preload_libraries".to_owned(), "a".to_owned()),
            ]
        );
        assert_eq!(preload_settings("x", 5, 9), None);
    }

    /// #458 review: after a cleanup that failed, the advice to drop or
    /// recreate scratch applies only to a database nothing else uses.
    #[test]
    fn cleanup_advice_keeps_a_shared_scratch_database() {
        let d = Diagnostic::new("DB_SCRATCH_CLEANUP", "x");
        assert!(
            d.next_action
                .contains("only this configuration's scratch database"),
            "{}",
            d.next_action
        );
        assert!(
            d.next_action.contains("leave this one as it is"),
            "{}",
            d.next_action
        );
    }

    /// #430 round 2: a cleanup that gave up on a lock names each session
    /// that holds one, and says how the next replay recovers.
    #[test]
    fn a_blocked_cleanup_names_the_sessions_that_hold_locks() {
        let d = cleanup_blocked(
            &[
                (7, Some("app_replay".into()), Some(true)),
                (8, Some("postgres".into()), Some(false)),
                (9, None, None),
            ],
            1,
        );
        assert_eq!(d.code, "DB_SCRATCH_CLEANUP");
        for part in [
            "55P03",
            "3 other session(s)",
            "pid 7 (the scratch role app_replay)",
            "pid 8 (role postgres)",
            "pid 9 (a background process, such as autovacuum)",
            "_ntnt_replay remain",
        ] {
            assert!(d.message.contains(part), "{part}: {}", d.message);
        }
        assert!(d.next_action.contains("rerun plan, status or verify"));
        assert!(d.next_action.contains("DB_SCRATCH_BUSY"));
        let gone = cleanup_blocked(&[], 3);
        assert!(gone.message.contains("3 time(s)"), "{}", gone.message);
        assert!(
            gone.message.contains("already released it"),
            "{}",
            gone.message
        );
        assert!(!gone.next_action.contains("DB_SCRATCH_BUSY"));
    }
}
