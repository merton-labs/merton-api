use std::time::Duration;

use merton_api::app::AppState;
use merton_api::config::DatabaseConfig;
use merton_api::infrastructure::database;
use merton_api::infrastructure::migrations::Migrator;
use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;

/// Connects to a fresh in-memory SQLite database with migrations applied.
///
/// A single connection is used because each SQLite in-memory connection is
/// its own database.
pub async fn test_db() -> Result<DatabaseConnection, DbErr> {
    let config = DatabaseConfig {
        url: "sqlite::memory:".to_owned(),
        max_connections: 1,
        busy_timeout: Duration::from_secs(1),
    };
    let db = database::connect(&config).await?;
    Migrator::up(&db, None).await?;
    Ok(db)
}

#[allow(dead_code)]
pub async fn test_state() -> Result<AppState, DbErr> {
    Ok(AppState {
        db: test_db().await?,
    })
}
