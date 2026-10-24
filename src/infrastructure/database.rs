//! Database connection setup.
//!
//! Backend-specific tuning is confined to this module; the rest of the
//! application only sees a SeaORM [`DatabaseConnection`].

use std::time::Duration;

use sea_orm::sqlx::sqlite::SqliteJournalMode;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr, RuntimeErr, SqlErr};

use crate::config::DatabaseConfig;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
/// `SQLITE_CONSTRAINT_TRIGGER`: SQLite enforces `ON DELETE RESTRICT` with an
/// internal trigger and reports this code instead of the usual FK code (787).
const SQLITE_CONSTRAINT_TRIGGER: &str = "1811";

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

/// Whether `err` is a foreign-key violation, including SQLite's
/// `ON DELETE RESTRICT` refusal, which SeaORM does not classify.
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
    sqlx_err.as_database_error().is_some_and(|db| {
        db.code().as_deref() == Some(SQLITE_CONSTRAINT_TRIGGER)
            && db.message().contains("FOREIGN KEY")
    })
}
