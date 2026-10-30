//! Two concrete native adapters, one plan/history/authorization/execution core.
//! This is intentionally not a plugin or portable-SQL framework.
use super::inventory::{Endpoint, Migration};
use super::model::{Catalog, History};
use super::postgres::{self, Postgres, RelationalAdapter};
use super::sqlite::{self, Sqlite};
use super::{Diagnostic, Result};
use serde::Serialize;
use serde_json::Value;

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
    pub async fn connect(
        secret: &crate::interpreter::SecretValue,
        endpoint: &Endpoint,
        engine: &str,
        access: Access,
    ) -> Result<Self> {
        match engine {
            "postgresql" => Postgres::connect(secret, endpoint)
                .await
                .map(Self::Postgres),
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
    pub async fn catalog(&self, schema: &str, role: &str) -> Result<Catalog> {
        match self {
            Self::Postgres(pg) => pg.catalog(schema, role).await,
            Self::Sqlite(db) => db.catalog().await,
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
    /// PostgreSQL owns the session across commits. SQLite reserves the native
    /// writer in begin_migration and revalidates on *every* acquisition instead.
    pub async fn own_migrations(&self) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.own_migrations().await,
            Self::Sqlite(_) => Ok(()),
        }
    }
    pub async fn begin_migration(&self, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.begin_migration(schema).await,
            Self::Sqlite(db) => db.begin_migration().await,
        }
    }
    pub async fn validate_execution_scope(&self, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.validate_execution_scope(schema).await,
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
    pub async fn prepare_scratch(
        &self,
        target: &Identity,
        endpoint: &Endpoint,
        schema: &str,
    ) -> Result<Identity> {
        match (self, target) {
            (Self::Postgres(pg), Identity::Postgres(target)) => pg
                .prepare_scratch(target, endpoint, schema)
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
    pub async fn execute_captured(&self, migration: &Migration, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.execute_captured(migration, schema).await,
            Self::Sqlite(db) => db.execute_captured(migration).await,
        }
    }
    pub async fn check_purity(&self, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.check_purity(schema).await,
            Self::Sqlite(db) => db.check_purity().await,
        }
    }
    pub async fn check(&self, sql: &str, schema: &str) -> Result<()> {
        match self {
            Self::Postgres(pg) => pg.check(sql, schema).await,
            Self::Sqlite(db) => db.check(sql).await,
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
