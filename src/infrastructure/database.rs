//! Database connection setup.
//!
//! Backend-specific tuning is confined to this module; the rest of the
//! application only sees a SeaORM [`DatabaseConnection`].

use std::time::Duration;

use sea_orm::sqlx::sqlite::SqliteJournalMode;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};

use crate::config::DatabaseConfig;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

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
