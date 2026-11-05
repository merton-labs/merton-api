//! Database connection setup.
//!
//! Backend-specific tuning is confined to this module; the rest of the
//! application only sees a SeaORM [`DatabaseConnection`].

use std::time::Duration;

use sea_orm::sqlx::sqlite::SqliteJournalMode;
use sea_orm::{
    AccessMode, ConnectOptions, Database, DatabaseConnection, DatabaseTransaction, DbBackend,
    DbErr, IsolationLevel, RuntimeErr, SqlErr, TransactionTrait,
};

use crate::config::DatabaseConfig;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
/// `SQLITE_CONSTRAINT_TRIGGER`: SQLite enforces `ON DELETE RESTRICT` with an
/// internal trigger and reports this code instead of the usual FK code (787).
const SQLITE_CONSTRAINT_TRIGGER: &str = "1811";
/// PostgreSQL's `restrict_violation`: `ON DELETE RESTRICT` is reported with
/// this code instead of `foreign_key_violation` (23503).
const POSTGRES_RESTRICT_VIOLATION: &str = "23001";

/// Opens a connection pool for the configured database.
pub async fn connect(config: &DatabaseConfig) -> Result<DatabaseConnection, DbErr> {
    let busy_timeout = config.busy_timeout;

    let mut options = ConnectOptions::new(config.url.clone());
    options
        .max_connections(config.max_connections)
        .min_connections(1)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        // Only applied when the URL selects SQLite.
        .map_sqlx_sqlite_opts(move |sqlite| {
            sqlite
                .create_if_missing(true)
                .foreign_keys(true)
                .busy_timeout(busy_timeout)
                .journal_mode(SqliteJournalMode::Wal)
        });

    Database::connect(options).await
}

/// Begins a read-only transaction whose statements all see one snapshot.
///
/// SQLite already reads a whole transaction from one snapshot. PostgreSQL's
/// default (`READ COMMITTED`) takes a new snapshot per statement, so a count
/// and its page, or holdings and their prices, could disagree; there this
/// asks for `REPEATABLE READ`.
pub async fn begin_read(db: &DatabaseConnection) -> Result<DatabaseTransaction, DbErr> {
    match db.get_database_backend() {
        DbBackend::Postgres => {
            db.begin_with_config(
                Some(IsolationLevel::RepeatableRead),
                Some(AccessMode::ReadOnly),
            )
            .await
        }
        _ => db.begin().await,
    }
}

/// Whether `err` is a foreign-key violation, including the
/// `ON DELETE RESTRICT` refusals of SQLite and PostgreSQL, which SeaORM does
/// not classify.
pub fn is_foreign_key_violation(err: &DbErr) -> bool {
    if matches!(
        err.sql_err(),
        Some(SqlErr::ForeignKeyConstraintViolation(_))
    ) {
        return true;
    }
    let (DbErr::Exec(RuntimeErr::SqlxError(sqlx_err))
    | DbErr::Query(RuntimeErr::SqlxError(sqlx_err))) = err
    else {
        return false;
    };
    sqlx_err
        .as_database_error()
        .is_some_and(|db| match db.code().as_deref() {
            Some(SQLITE_CONSTRAINT_TRIGGER) => db.message().contains("FOREIGN KEY"),
            Some(POSTGRES_RESTRICT_VIOLATION) => true,
            _ => false,
        })
}
