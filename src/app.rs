//! Application wiring: database, migrations, HTTP server and shutdown.

use std::net::SocketAddr;

use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::http;
use crate::infrastructure::database;
use crate::infrastructure::migrations::Migrator;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("failed to connect to database: {0}")]
    DatabaseConnect(#[source] DbErr),
    #[error("failed to run database migrations: {0}")]
    Migration(#[source] DbErr),
    #[error("failed to bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("server error: {0}")]
    Serve(#[source] std::io::Error),
}

/// State shared with HTTP handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
}

/// Connects to the database, applies migrations and serves HTTP until a
/// shutdown signal is received.
pub async fn run(config: Config) -> Result<(), ServerError> {
    let db = database::connect(&config.database)
        .await
        .map_err(ServerError::DatabaseConnect)?;
    tracing::info!(
        backend = ?db.get_database_backend(),
        max_connections = config.database.max_connections,
        "database connected"
    );

    Migrator::up(&db, None)
        .await
        .map_err(ServerError::Migration)?;
    tracing::info!("database migrations applied");

    let addr = config.server.socket_addr();
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| ServerError::Bind { addr, source })?;
    tracing::info!(%addr, "listening");

    let router = http::router(AppState { db: db.clone() });
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(ServerError::Serve)?;

    if let Err(err) = db.close().await {
        tracing::warn!(error = %err, "failed to close database connections");
    }
    tracing::info!("shutdown complete");
    Ok(())
}

/// Resolves on Ctrl+C or, on Unix, SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to listen for Ctrl+C");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
