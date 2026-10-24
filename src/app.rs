//! Application wiring: database, migrations, HTTP server and shutdown.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;
use tokio::net::TcpListener;

use crate::auth::csrf::{CsrfKey, InvalidKey};
use crate::auth::rate_limit::LoginRateLimiter;
use crate::config::{Config, SecurityConfig};
use crate::http;
use crate::infrastructure::database;
use crate::infrastructure::migrations::Migrator;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("invalid security configuration: {0}")]
    Security(#[from] InvalidKey),
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
    pub security: Arc<Security>,
}

/// Process-wide security state for request handling.
pub struct Security {
    pub csrf: CsrfKey,
    pub login_limiter: LoginRateLimiter,
    pub trusted_origins: Vec<String>,
    pub trusted_proxies: Vec<IpAddr>,
    pub cookie_secure: bool,
}

impl AppState {
    pub fn new(db: DatabaseConnection, config: &SecurityConfig) -> Result<Self, InvalidKey> {
        Ok(Self {
            db,
            security: Arc::new(Security {
                csrf: CsrfKey::new(&config.secret_key)?,
                login_limiter: LoginRateLimiter::default(),
                trusted_origins: config.trusted_origins.clone(),
                trusted_proxies: config.trusted_proxies.clone(),
                cookie_secure: config.cookie_secure,
            }),
        })
    }
}

/// Connects to the database, applies migrations and serves HTTP until a
/// shutdown signal is received.
pub async fn run(config: Config) -> Result<(), ServerError> {
    tracing::info!(
        trusted_origins = ?config.security.trusted_origins,
        trusted_proxies = ?config.security.trusted_proxies,
        cookie_secure = config.security.cookie_secure,
        "security configuration loaded"
    );
    if !config.security.cookie_secure {
        tracing::warn!("SESSION_COOKIE_SECURE=false: session cookies are sent over plain HTTP");
    }

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

    let state = AppState::new(db.clone(), &config.security)?;
    let router = http::router(state).into_make_service_with_connect_info::<SocketAddr>();
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
