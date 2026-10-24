use std::io::IsTerminal;
use std::process::ExitCode;

use merton_api::app;
use merton_api::config::Config;
use tracing_subscriber::EnvFilter;

const DEFAULT_LOG_FILTER: &str = "merton_api=info,tower_http=info,sqlx=warn";

// The runtime is built by hand so `.env` is loaded before any worker threads exist.
fn main() -> ExitCode {
    let dotenv = dotenvy::dotenv();
    init_tracing();
    if let Err(err) = dotenv
        && !err.not_found()
    {
        tracing::warn!(error = %err, "failed to load .env file");
    }

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            tracing::error!(error = %err, "invalid configuration");
            return ExitCode::FAILURE;
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::error!(error = %err, "failed to start async runtime");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(app::run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(std::io::stdout().is_terminal())
        .init();
}
