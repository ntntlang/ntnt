//! Two concrete native adapters, one plan/history/authorization/execution core.
//! This is intentionally not a plugin or portable-SQL framework.
use super::inventory::{CatalogDigests, Endpoint, Fingerprint, Migration};
use super::model::{Catalog, History};
use super::postgres::{self, Postgres, RelationalAdapter};
use super::sqlite::{self, Sqlite};
use super::{Diagnostic, Result};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub(super) enum Access {
    Inspect,
    Execute(Value),
    Scratch(Value),
}
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub(super) enum Identity {
    Postgres(postgres::Identity),
    Sqlite(sqlite::Identity),
}
impl Identity {
    pub fn capabilities(&self) -> Value {
        match self {
            Self::Postgres(i) => Postgres::capabilities(i.version),
            Self::Sqlite(i) => Sqlite::capabilities(i.version),
        }
    }
    /// Whether row-level security can apply to this login role. SQLite has
    /// none; a PostgreSQL role escapes it with BYPASSRLS or as a superuser.
    pub fn row_security_applies(&self) -> bool {
        match self {
            Self::Postgres(i) => !i.bypasses_row_security(),
            Self::Sqlite(_) => false,
        }
    }
}
/// The descriptor format identifies an engine-specific canonical catalog.
pub(super) fn catalog_format(engine: &str) -> &'static str {
    match engine {
        "sqlite" => "sqlite-catalog-v1",
        _ => "postgresql-catalog-v1",
    }
}
pub(super) fn execution_format(engine: &str) -> &'static str {
    match engine {
        "sqlite" => "sqlite-transactional-v1",
        _ => "postgresql-transactional-v1",
    }
}

pub(super) enum Adapter {
    Postgres(Postgres),
    Sqlite(Sqlite),
}
impl Adapter {
    /// `extensions` are the declared PostgreSQL extensions
    /// (`Database::declared_extensions`), which this session's inspection
    /// covers wherever they are installed; empty for SQLite.
    pub async fn connect(
        secret: &crate::interpreter::SecretValue,
        endpoint: &Endpoint,
        engine: &str,
        access: Access,
        extensions: &[String],
    ) -> Result<Self> {
        match engine {
            "postgresql" => Postgres::connect(secret, endpoint)
                .await
                .map(|pg| Self::Postgres(pg.with_declared_extensions(extensions))),
            "sqlite" => Sqlite::connect(secret, access).await.map(Self::Sqlite),
            _ => Err(Diagnostic::new(
                "DB_ENGINE",
                "Unsupported database adapter.",
            )),
        }
    }
    pub async fn identity(&self, endpoint: &Endpoint) -> Result<Identity> {
        match self {
            Self::Postgres(pg) => pg.identity(endpoint).await.map(Identity::Postgres),
            Self::Sqlite(db) => db.identity().await.map(Identity::Sqlite),
        }
    }
    /// The managed scope's catalog: `schema` and, for PostgreSQL, the `extra`
    /// schemas the history creates (`CapturedInputs::history_schemas`) and the
    /// `outside` objects it creates in other schemas, by catalog key (empty
    /// for SQLite).
    pub async fn catalog(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
    ) -> Result<Catalog> {
        match self {
            Self::Postgres(pg) => pg.catalog(schema, extra, outside, role).await,
            Self::Sqlite(db) => db.catalog().await,
        }
    }
    /// `catalog().uncovered` and `Fingerprint::of(&catalog())`, without
    /// keeping the catalog. SQLite fingerprints the catalog it maintains
    /// itself, whose values are all stored restore-stable, and leaves
    /// `digests` unused.
    pub async fn fingerprint(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
        digests: &mut CatalogDigests,
    ) -> Result<(Vec<String>, Fingerprint)> {
        match self {
            Self::Postgres(pg) => {
                pg.live_fingerprint(schema, extra, outside, role, digests)
                    .await
            }
            Self::Sqlite(db) => db.fingerprint().await.map(|(uncovered, stable)| {
                (
                    uncovered,
                    Fingerprint {
                        stable,
                        stored: None,
                    },
                )
            }),
        }
    }
    /// `catalog().uncovered`, without keeping the catalog.
    pub async fn coverage(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
        role: &str,
    ) -> Result<Vec<String>> {
        match self {
            Self::Postgres(pg) => Ok(pg.catalog(schema, extra, outside, role).await?.uncovered),
            Self::Sqlite(db) => db.coverage().await,
        }
    }
    /// PostgreSQL scratch: what the replayed history created outside the
    /// managed scope so far, by catalog key, for `catalog`'s `outside`.
    /// Empty for SQLite, whose scope is all of `main`, and for a target.
    pub async fn outside_objects(&self, schema: &str, extra: &[String]) -> Result<Vec<String>> {
        match self {
            Self::Postgres(pg) => pg.outside_objects(schema, extra).await,
            Self::Sqlite(_) => Ok(vec![]),
        }
    }
    pub async fn history(&self) -> Result<History> {
        match self {
            Self::Postgres(pg) => pg.history().await,
            Self::Sqlite(db) => db.history(None).await,
        }
    }
    pub async fn writable_history(&self, database: &str, schema: &str) -> Result<History> {
        match self {
            Self::Postgres(pg) => pg.writable_history(database, schema).await,
            Self::Sqlite(db) => db.history(Some(database)).await,
        }
    }
    pub async fn begin_readonly(&self) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.begin_readonly().await,
            Self::Sqlite(db) => db.begin_readonly().await,
        }
    }
    /// PostgreSQL owns the session across commits, waiting up to `wait` for
    /// another migrator. SQLite reserves the native writer in begin_migration
    /// and revalidates on *every* acquisition instead.
    pub async fn own_migrations(&self, wait: Duration) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.own_migrations(wait).await,
            Self::Sqlite(_) => Ok(()),
        }
    }
    pub async fn begin_migration(&self, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.begin_migration(schema).await,
            Self::Sqlite(db) => db.begin_migration().await,
        }
    }
    /// PostgreSQL: SHARE-lock tables that checks read. SQLite already holds
    /// the database writer lock from BEGIN IMMEDIATE.
    pub async fn lock_for_checks(&self, tables: &[(String, String)]) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.lock_for_checks(tables).await,
            Self::Sqlite(_) => Ok(()),
        }
    }
    pub async fn index_validity(&self, schema: &str, name: &str) -> Result<Option<bool>> {
        match self {
            Self::Postgres(pg) => pg.index_validity(schema, name).await,
            Self::Sqlite(_) => Ok(None),
        }
    }
    pub async fn build_concurrent_index(
        &self,
        migration: &Migration,
        schema: &str,
        extra: &[String],
        limit: Duration,
    ) -> Result<()> {
        match self {
            Self::Postgres(pg) => {
                pg.build_concurrent_index(migration, schema, extra, limit)
                    .await
            }
            Self::Sqlite(_) => Err(Diagnostic::new(
                "DB_REPLAY_UNSUPPORTED",
                "SQLite has no concurrent index builds.",
            )),
        }
    }
    pub async fn index_building(&self, schema: &str, name: &str) -> Result<bool> {
        match self {
            Self::Postgres(pg) => pg.index_building(schema, name).await,
            Self::Sqlite(_) => Ok(false),
        }
    }
    /// Record a non-transactional migration as `running` before it starts.
    pub async fn record_attempt(
        &self,
        migration: &Migration,
        position: usize,
        database: &str,
        schema: &str,
        present: bool,
    ) -> Result<()> {
        let entry = super::model::HistoryEntry {
            position: position as i64,
            id: migration.id.to_string(),
            name: migration.name.clone(),
            checksum: migration.checksum.clone(),
            state: "running".into(),
        };
        match self {
            Self::Postgres(pg) => pg.record_entry(&entry, database, schema, present).await,
            Self::Sqlite(_) => Err(unsupported_attempt()),
        }
    }
    pub async fn resolve_attempt(&self, position: usize, applied: bool) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.resolve_attempt(position, applied).await,
            Self::Sqlite(_) => Err(unsupported_attempt()),
        }
    }
    pub async fn drop_invalid_index(
        &self,
        schema: &str,
        name: &str,
        limit: Duration,
    ) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.drop_invalid_index(schema, name, limit).await,
            Self::Sqlite(_) => Ok(()),
        }
    }
    /// Locks the current migration transaction holds on managed tables.
    pub async fn relation_locks(&self, schema: &str) -> Result<Option<Value>> {
        match self {
            Self::Postgres(pg) => pg.relation_locks(schema).await.map(Some),
            Self::Sqlite(_) => Ok(None),
        }
    }
    pub async fn validate_execution_scope(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
    ) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.validate_execution_scope(schema, extra, outside).await,
            Self::Sqlite(_) => Ok(()),
        }
    }
    pub async fn rollback(&self) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.rollback().await,
            Self::Sqlite(db) => db.rollback().await,
        }
    }
    pub async fn commit(&self) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.batch("COMMIT").await,
            Self::Sqlite(db) => db.commit().await,
        }
    }
    /// PostgreSQL waits up to `wait` for another replay of the same scratch
    /// database and commits each replayed migration, so the caller must end
    /// with `cleanup_scratch`, whatever happens. SQLite keeps one rollback-only
    /// transaction.
    pub async fn prepare_scratch(
        &self,
        target: &Identity,
        endpoint: &Endpoint,
        schema: &str,
        wait: Duration,
    ) -> Result<Identity> {
        match (self, target) {
            (Self::Postgres(pg), Identity::Postgres(target)) => pg
                .prepare_scratch(target, endpoint, schema, wait)
                .await
                .map(Identity::Postgres),
            (Self::Sqlite(db), Identity::Sqlite(target)) => {
                db.prepare_scratch(target).await.map(Identity::Sqlite)
            }
            _ => Err(Diagnostic::new(
                "DB_SCRATCH_MISMATCH",
                "Scratch and target engines differ.",
            )),
        }
    }
    /// PostgreSQL: the replay whose leftovers `prepare_scratch` cleaned up.
    pub fn scratch_recovered(&self) -> Option<Value> {
        match self {
            Self::Postgres(pg) => pg.scratch_recovered(),
            Self::Sqlite(_) => None,
        }
    }
    /// PostgreSQL opens the transaction one replayed migration commits in,
    /// set up as the target's; SQLite replays in its one transaction.
    pub async fn begin_replay_step(&self) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.begin_replay_step().await,
            Self::Sqlite(_) => Ok(()),
        }
    }
    /// PostgreSQL refuses what `migration` did that cleanup could not undo,
    /// then commits it.
    pub async fn finish_replay_step(&self, migration: Option<&Migration>) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.finish_replay_step(migration).await,
            Self::Sqlite(_) => Ok(()),
        }
    }
    /// After replay, on every exit: PostgreSQL drops what the replay
    /// committed in scratch and checks it is empty again, false when it had
    /// committed nothing, then releases its replay lock, whose failure comes
    /// back beside cleanup's outcome; SQLite rolls back.
    pub async fn cleanup_scratch(&self) -> (Result<bool>, Option<Diagnostic>) {
        match self {
            Self::Postgres(pg) => pg.cleanup_scratch().await,
            Self::Sqlite(db) => (db.rollback().await.map(|()| true), None),
        }
    }
    /// PostgreSQL cancels the statement the scratch session is running,
    /// which replay's deadline interrupted, before cleanup.
    pub async fn cancel_running(&self) {
        if let Self::Postgres(pg) = self {
            pg.cancel_running().await;
        }
    }
    /// `limit` bounds the migration on a target: each PostgreSQL statement, or
    /// the whole SQLite script. Scratch replay keeps SQLite's fixed bound.
    /// `extra` are the schemas the history creates (PostgreSQL only).
    pub async fn execute_captured(
        &self,
        migration: &Migration,
        schema: &str,
        extra: &[String],
        limit: Duration,
    ) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.execute_captured(migration, schema, extra, limit).await,
            Self::Sqlite(db) => db.execute_captured(migration, limit).await,
        }
    }
    /// Live objects must stay within check-purity coverage. `outside` lists
    /// the objects the history creates outside the managed scope, which the
    /// plan records; references to them are covered as in every other
    /// inspection (#420).
    pub async fn check_purity(
        &self,
        schema: &str,
        extra: &[String],
        outside: &[String],
    ) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.check_purity(schema, extra, outside).await,
            Self::Sqlite(db) => db.check_purity().await,
        }
    }
    /// PostgreSQL: what stops check `sql` from running as written
    /// (`Postgres::check_hazards`). SQLite resolves no operators by search
    /// path, and its authorizer already refuses every function a view could
    /// call to write.
    pub async fn check_hazards(&self, schema: &str, sql: &str) -> Result<Vec<super::CheckHazard>> {
        match self {
            Self::Postgres(pg) => pg.check_hazards(schema, sql).await,
            Self::Sqlite(_) => Ok(vec![]),
        }
    }
    pub async fn check(&self, sql: &str, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.check(sql, schema).await,
            Self::Sqlite(db) => db.check(sql).await,
        }
    }
    /// PostgreSQL: what a check naming `tables` would read where row-level
    /// security applies to this session's role, which a check cannot read
    /// (`Postgres::row_security_blocked`). SQLite has no row-level security.
    pub async fn row_security_blocked(
        &self,
        schema: &str,
        tables: &[String],
    ) -> Result<Vec<super::RowSecurity>> {
        match self {
            Self::Postgres(pg) => pg.row_security_blocked(schema, tables).await,
            Self::Sqlite(_) => Ok(vec![]),
        }
    }
    pub async fn import_legacy_history(&self, database: &str, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.import_legacy_history(database, schema).await,
            Self::Sqlite(_) => Err(Diagnostic::new(
                "DB_ADOPTION_INPUT",
                "SQLite has no supported legacy ledger import.",
            )),
        }
    }
    pub async fn record_checkpoint(
        &self,
        entry: &super::model::HistoryEntry,
        database: &str,
        schema: &str,
    ) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.record_entry(entry, database, schema, false).await,
            Self::Sqlite(db) => db.record_entry(entry, database, false).await,
        }
    }
    pub async fn record_migration(
        &self,
        migration: &Migration,
        position: usize,
        database: &str,
        schema: &str,
        present: bool,
    ) -> Result<()> {
        match self {
            Self::Postgres(pg) => {
                pg.record_migration(migration, position, database, schema, present)
                    .await
            }
            Self::Sqlite(db) => {
                db.record_migration(migration, position, database, present)
                    .await
            }
        }
    }
}

fn unsupported_attempt() -> Diagnostic {
    Diagnostic::new(
        "DB_REPLAY_UNSUPPORTED",
        "SQLite has no non-transactional migrations.",
    )
}
