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
const DEFAULT_TRUSTED_ORIGINS: &str = "http://localhost:8080,http://127.0.0.1:8080";
const MIN_SECRET_KEY_BYTES: usize = 32;
const DEFAULT_MAX_CONCURRENT_HASHES: usize = 4;
/// Each Argon2 operation holds ~19 MiB, so 64 caps hashing memory at ~1.2 GiB.
const MAX_CONCURRENT_HASHES_LIMIT: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("environment variable {name} is required")]
    Missing { name: &'static str },
    #[error("environment variable {name} has invalid value {value:?}: {reason}")]
    InvalidValue {
        name: &'static str,
        value: String,
        reason: String,
    },
    // Never echoes the value.
    #[error("environment variable {name} is invalid: {reason}")]
    InvalidSecret { name: &'static str, reason: String },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub security: SecurityConfig,
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

#[derive(Clone)]
pub struct SecurityConfig {
    /// Server secret used to derive CSRF tokens. Must persist across restarts.
    pub secret_key: Vec<u8>,
    /// Exact origins (`scheme://host[:port]`) the browser may send mutating requests from.
    pub trusted_origins: Vec<String>,
    /// Reverse proxies whose `X-Forwarded-For` header is believed.
    pub trusted_proxies: Vec<IpAddr>,
    /// Whether session cookies carry `Secure` (and the `__Host-` prefix).
    pub cookie_secure: bool,
    /// Maximum concurrent Argon2 operations; more are rejected with 503.
    pub max_concurrent_hashes: usize,
}

impl fmt::Debug for SecurityConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecurityConfig")
            .field("secret_key", &"<redacted>")
            .field("trusted_origins", &self.trusted_origins)
            .field("trusted_proxies", &self.trusted_proxies)
            .field("cookie_secure", &self.cookie_secure)
            .field("max_concurrent_hashes", &self.max_concurrent_hashes)
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
            security: security_from(&lookup)?,
        })
    }
}

fn security_from(lookup: &impl Fn(&str) -> Option<String>) -> Result<SecurityConfig, ConfigError> {
    let secret_key = lookup("APP_SECRET_KEY")
        .map(|value| value.trim().as_bytes().to_vec())
        .ok_or(ConfigError::Missing {
            name: "APP_SECRET_KEY",
        })?;
    if secret_key.len() < MIN_SECRET_KEY_BYTES {
        return Err(ConfigError::InvalidSecret {
            name: "APP_SECRET_KEY",
            reason: format!("must be at least {MIN_SECRET_KEY_BYTES} bytes"),
        });
    }

    let origins = lookup("APP_TRUSTED_ORIGINS");
    let origins = origins.as_deref().unwrap_or(DEFAULT_TRUSTED_ORIGINS);
    let trusted_origins = parse_list(origins, "APP_TRUSTED_ORIGINS", parse_origin)?;

    let proxies = lookup("TRUSTED_PROXIES").unwrap_or_default();
    let trusted_proxies = parse_list(&proxies, "TRUSTED_PROXIES", |item| {
        item.parse::<IpAddr>().map_err(|err| err.to_string())
    })?;

    let cookie_secure = parse_or(&lookup, "SESSION_COOKIE_SECURE", true)?;
    if !cookie_secure && !trusted_origins.iter().all(|o| is_loopback_http_origin(o)) {
        return Err(ConfigError::InvalidValue {
            name: "SESSION_COOKIE_SECURE",
            value: "false".to_owned(),
            reason: "insecure cookies are only allowed when every trusted origin is \
                     http://localhost, http://127.0.0.1 or http://[::1]"
                .to_owned(),
        });
    }

    let max_concurrent_hashes = parse_or(
        &lookup,
        "AUTH_MAX_CONCURRENT_HASHES",
        DEFAULT_MAX_CONCURRENT_HASHES,
    )?;
    if !(1..=MAX_CONCURRENT_HASHES_LIMIT).contains(&max_concurrent_hashes) {
        return Err(ConfigError::InvalidValue {
            name: "AUTH_MAX_CONCURRENT_HASHES",
            value: max_concurrent_hashes.to_string(),
            reason: format!("must be between 1 and {MAX_CONCURRENT_HASHES_LIMIT}"),
        });
    }

    Ok(SecurityConfig {
        secret_key,
        trusted_origins,
        trusted_proxies,
        cookie_secure,
        max_concurrent_hashes,
    })
}

fn parse_list<T>(
    raw: &str,
    name: &'static str,
    parse_item: impl Fn(&str) -> Result<T, String>,
) -> Result<Vec<T>, ConfigError> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            parse_item(item).map_err(|reason| ConfigError::InvalidValue {
                name,
                value: item.to_owned(),
                reason,
            })
        })
        .collect()
}

/// Parses an origin into the form browsers send in the `Origin` header:
/// lowercase, no trailing slash, no default port.
fn parse_origin(raw: &str) -> Result<String, String> {
    let lower = raw.trim_end_matches('/').to_ascii_lowercase();
    let (scheme, authority) = lower
        .split_once("://")
        .ok_or("must look like scheme://host[:port]")?;
    let default_port = match scheme {
        "http" => ":80",
        "https" => ":443",
        _ => return Err("scheme must be http or https".to_owned()),
    };
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', '*']) {
        return Err("must be an origin without path, query or wildcard".to_owned());
    }
    let authority = authority.strip_suffix(default_port).unwrap_or(authority);
    Ok(format!("{scheme}://{authority}"))
}

fn is_loopback_http_origin(origin: &str) -> bool {
    let Some(authority) = origin.strip_prefix("http://") else {
        return false;
    };
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => host,
        _ => authority,
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
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

    const SECRET: &str = "test-secret-key-that-is-long-enough-123";

    fn config_from(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let mut vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        vars.entry("APP_SECRET_KEY".to_owned())
            .or_insert_with(|| SECRET.to_owned());
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
        assert_eq!(
            config.security.trusted_origins,
            ["http://localhost:8080", "http://127.0.0.1:8080"]
        );
        assert!(config.security.trusted_proxies.is_empty());
        assert!(config.security.cookie_secure);
        assert_eq!(config.security.max_concurrent_hashes, 4);
    }

    #[test]
    fn reads_overrides() {
        let config = config_from(&[
            ("APP_HOST", "0.0.0.0"),
            ("APP_PORT", "3000"),
            ("DATABASE_URL", "sqlite::memory:"),
            ("DATABASE_MAX_CONNECTIONS", "2"),
            ("DATABASE_BUSY_TIMEOUT_MS", "250"),
            ("APP_TRUSTED_ORIGINS", "https://Merton.Example.com:443/"),
            ("TRUSTED_PROXIES", "10.0.0.2, ::1"),
            ("AUTH_MAX_CONCURRENT_HASHES", "8"),
        ])
        .unwrap();

        assert_eq!(config.server.socket_addr(), "0.0.0.0:3000".parse().unwrap());
        assert_eq!(config.database.url, "sqlite::memory:");
        assert_eq!(config.database.max_connections, 2);
        assert_eq!(config.database.busy_timeout, Duration::from_millis(250));
        assert_eq!(
            config.security.trusted_origins,
            ["https://merton.example.com"]
        );
        assert_eq!(
            config.security.trusted_proxies,
            [
                "10.0.0.2".parse::<IpAddr>().unwrap(),
                "::1".parse().unwrap()
            ]
        );
        assert_eq!(config.security.max_concurrent_hashes, 8);
    }

    #[test]
    fn rejects_out_of_range_hash_concurrency() {
        for value in ["0", "65", "-1", "many"] {
            let err = config_from(&[("AUTH_MAX_CONCURRENT_HASHES", value)]).unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::InvalidValue {
                        name: "AUTH_MAX_CONCURRENT_HASHES",
                        ..
                    }
                ),
                "{value}"
            );
        }
        assert!(config_from(&[("AUTH_MAX_CONCURRENT_HASHES", "64")]).is_ok());
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
    fn requires_a_long_enough_secret_without_echoing_it() {
        let vars = HashMap::<String, String>::new();
        let missing = Config::from_lookup(|name| vars.get(name).cloned()).unwrap_err();
        assert!(matches!(
            missing,
            ConfigError::Missing {
                name: "APP_SECRET_KEY"
            }
        ));

        let short = config_from(&[("APP_SECRET_KEY", "hunter2-short")]).unwrap_err();
        assert!(matches!(short, ConfigError::InvalidSecret { .. }));
        assert!(!short.to_string().contains("hunter2"));
    }

    #[test]
    fn rejects_invalid_origins() {
        for origin in [
            "localhost:8080",
            "ftp://example.com",
            "https://example.com/app",
            "*",
            "https://*",
        ] {
            let err = config_from(&[("APP_TRUSTED_ORIGINS", origin)]).unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::InvalidValue {
                        name: "APP_TRUSTED_ORIGINS",
                        ..
                    }
                ),
                "{origin}"
            );
        }
    }

    #[test]
    fn insecure_cookies_only_for_loopback_http() {
        let local = config_from(&[
            ("SESSION_COOKIE_SECURE", "false"),
            (
                "APP_TRUSTED_ORIGINS",
                "http://localhost:3000,http://[::1]:8080",
            ),
        ]);
        assert!(local.is_ok_and(|c| !c.security.cookie_secure));

        for origins in ["https://merton.example.com", "http://192.168.1.10:8080"] {
            let err = config_from(&[
                ("SESSION_COOKIE_SECURE", "false"),
                ("APP_TRUSTED_ORIGINS", origins),
            ])
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::InvalidValue {
                        name: "SESSION_COOKIE_SECURE",
                        ..
                    }
                ),
                "{origins}"
            );
        }
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let config = config_from(&[("DATABASE_URL", "postgres://user:secret@db/merton")]).unwrap();
        let debug = format!("{config:?}");

        assert!(!debug.contains("secret@"));
        assert!(!debug.contains(SECRET));
    }
}
