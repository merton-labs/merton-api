//! Application configuration loaded from environment variables.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_DATABASE_URL: &str = "sqlite://data/merton.db";
const DEFAULT_DATABASE_MAX_CONNECTIONS: u32 = 5;
const DEFAULT_DATABASE_BUSY_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("environment variable {name} has invalid value {value:?}: {reason}")]
    InvalidValue {
        name: &'static str,
        value: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: IpAddr,
    pub port: u16,
}

impl ServerConfig {
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

#[derive(Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub busy_timeout: Duration,
}

// The URL may carry credentials (e.g. a future PostgreSQL DSN), so it is never printed.
impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("url", &"<redacted>")
            .field("max_connections", &self.max_connections)
            .field("busy_timeout", &self.busy_timeout)
            .finish()
    }
}

impl Config {
    /// Reads configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads configuration using `lookup` to resolve variable names.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let max_connections = parse_or(
            &lookup,
            "DATABASE_MAX_CONNECTIONS",
            DEFAULT_DATABASE_MAX_CONNECTIONS,
        )?;
        if max_connections == 0 {
            return Err(ConfigError::InvalidValue {
                name: "DATABASE_MAX_CONNECTIONS",
                value: max_connections.to_string(),
                reason: "must be greater than zero".to_owned(),
            });
        }

        Ok(Self {
            server: ServerConfig {
                host: parse_or(&lookup, "APP_HOST", DEFAULT_HOST)?,
                port: parse_or(&lookup, "APP_PORT", DEFAULT_PORT)?,
            },
            database: DatabaseConfig {
                url: lookup("DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned()),
                max_connections,
                busy_timeout: Duration::from_millis(parse_or(
                    &lookup,
                    "DATABASE_BUSY_TIMEOUT_MS",
                    DEFAULT_DATABASE_BUSY_TIMEOUT_MS,
                )?),
            },
        })
    }
}

fn parse_or<T>(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: T,
) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    match lookup(name) {
        None => Ok(default),
        Some(value) => value
            .trim()
            .parse()
            .map_err(|err: T::Err| ConfigError::InvalidValue {
                name,
                reason: err.to_string(),
                value,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config_from(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn uses_defaults_when_unset() {
        let config = config_from(&[]).unwrap();

        assert_eq!(
            config.server.socket_addr(),
            "127.0.0.1:8080".parse().unwrap()
        );
        assert_eq!(config.database.url, DEFAULT_DATABASE_URL);
        assert_eq!(config.database.max_connections, 5);
        assert_eq!(config.database.busy_timeout, Duration::from_secs(5));
    }

    #[test]
    fn reads_overrides() {
        let config = config_from(&[
            ("APP_HOST", "0.0.0.0"),
            ("APP_PORT", "3000"),
            ("DATABASE_URL", "sqlite::memory:"),
            ("DATABASE_MAX_CONNECTIONS", "2"),
            ("DATABASE_BUSY_TIMEOUT_MS", "250"),
        ])
        .unwrap();

        assert_eq!(config.server.socket_addr(), "0.0.0.0:3000".parse().unwrap());
        assert_eq!(config.database.url, "sqlite::memory:");
        assert_eq!(config.database.max_connections, 2);
        assert_eq!(config.database.busy_timeout, Duration::from_millis(250));
    }

    #[test]
    fn rejects_invalid_port() {
        let err = config_from(&[("APP_PORT", "not-a-port")]).unwrap_err();

        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                name: "APP_PORT",
                ..
            }
        ));
    }

    #[test]
    fn rejects_zero_max_connections() {
        let err = config_from(&[("DATABASE_MAX_CONNECTIONS", "0")]).unwrap_err();

        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                name: "DATABASE_MAX_CONNECTIONS",
                ..
            }
        ));
    }

    #[test]
    fn debug_output_redacts_database_url() {
        let config = config_from(&[("DATABASE_URL", "postgres://user:secret@db/merton")]).unwrap();

        assert!(!format!("{config:?}").contains("secret"));
    }
}
