#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use merton_api::app::AppState;
use merton_api::config::{DatabaseConfig, SecurityConfig};
use merton_api::infrastructure::database;
use merton_api::infrastructure::migrations::Migrator;
use sea_orm::{DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;
use serde_json::Value;
use tower::ServiceExt;

pub const ORIGIN: &str = "http://localhost:8080";
pub const SESSION_COOKIE: &str = "__Host-merton_session";
pub const PASSWORD: &str = "correct horse battery staple";

/// Connects to a fresh in-memory SQLite database with migrations applied.
///
/// A single connection is used because each SQLite in-memory connection is
/// its own database, which also keeps every test isolated.
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

pub fn security_config() -> SecurityConfig {
    SecurityConfig {
        secret_key: b"integration-test-secret-key-0123456789".to_vec(),
        trusted_origins: vec![ORIGIN.to_owned()],
        trusted_proxies: Vec::new(),
        cookie_secure: true,
        max_concurrent_hashes: 4,
    }
}

pub async fn test_state() -> Result<AppState, DbErr> {
    test_state_with(&security_config()).await
}

pub async fn test_state_with(config: &SecurityConfig) -> Result<AppState, DbErr> {
    Ok(AppState::new(test_db().await?, config).expect("valid test key"))
}

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl TestResponse {
    pub fn content_type(&self) -> &str {
        self.headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }

    /// The `Set-Cookie` header for the session cookie, if any.
    pub fn session_set_cookie(&self) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{SESSION_COOKIE}=")))
            .map(str::to_owned)
    }

    /// The session token from `Set-Cookie`.
    pub fn session_token(&self) -> Option<String> {
        let cookie = self.session_set_cookie()?;
        let value = cookie.split(';').next()?.split_once('=')?.1;
        (!value.is_empty()).then(|| value.to_owned())
    }

    pub fn assert_problem(&self, status: StatusCode, code: &str) {
        assert_eq!(self.status, status, "body: {}", self.body);
        assert_eq!(self.content_type(), "application/problem+json");
        assert_eq!(self.body["status"], status.as_u16());
        assert_eq!(self.body["code"], code, "body: {}", self.body);
        assert!(self.body["title"].is_string());
        assert!(self.body["type"].is_string());
    }
}

#[derive(Clone)]
pub struct TestApp {
    pub state: AppState,
    router: Router,
}

pub struct Req<'a> {
    method: Method,
    uri: &'a str,
    body: Option<Value>,
    session: Option<&'a str>,
    headers: Vec<(&'a str, &'a str)>,
    peer: SocketAddr,
}

impl<'a> Req<'a> {
    pub fn get(uri: &'a str) -> Self {
        Self::new(Method::GET, uri)
    }

    pub fn post(uri: &'a str) -> Self {
        Self::new(Method::POST, uri).header("origin", ORIGIN)
    }

    pub fn patch(uri: &'a str) -> Self {
        Self::new(Method::PATCH, uri).header("origin", ORIGIN)
    }

    pub fn delete(uri: &'a str) -> Self {
        Self::new(Method::DELETE, uri).header("origin", ORIGIN)
    }

    pub fn put(uri: &'a str) -> Self {
        Self::new(Method::PUT, uri).header("origin", ORIGIN)
    }

    fn new(method: Method, uri: &'a str) -> Self {
        Self {
            method,
            uri,
            body: None,
            session: None,
            headers: Vec::new(),
            peer: "127.0.0.1:50000".parse().unwrap(),
        }
    }

    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    pub fn session(mut self, token: &'a str) -> Self {
        self.session = Some(token);
        self
    }

    pub fn header(mut self, name: &'a str, value: &'a str) -> Self {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name, value));
        self
    }

    pub fn without_header(mut self, name: &str) -> Self {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self
    }

    pub fn peer(mut self, peer: &str) -> Self {
        self.peer = peer.parse().unwrap();
        self
    }
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config(&security_config()).await
    }

    pub async fn with_config(config: &SecurityConfig) -> Self {
        let state = test_state_with(config).await.unwrap();
        let router = merton_api::http::router(state.clone());
        Self { state, router }
    }

    pub fn db(&self) -> &DatabaseConnection {
        &self.state.db
    }

    pub async fn send(&self, req: Req<'_>) -> TestResponse {
        let mut builder = Request::builder().method(req.method).uri(req.uri);
        for (name, value) in &req.headers {
            builder = builder.header(*name, *value);
        }
        if let Some(token) = req.session {
            builder = builder.header(header::COOKIE, format!("{SESSION_COOKIE}={token}"));
        }
        let body = match req.body {
            Some(json) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let mut request = builder.body(body).unwrap();
        request.extensions_mut().insert(ConnectInfo(req.peer));

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        TestResponse {
            status,
            headers,
            body,
        }
    }

    pub async fn register(&self, email: &str, password: &str) -> TestResponse {
        self.send(
            Req::post("/api/v1/auth/register")
                .json(serde_json::json!({ "email": email, "password": password })),
        )
        .await
    }

    pub async fn login(&self, email: &str, password: &str) -> TestResponse {
        self.send(
            Req::post("/api/v1/auth/login")
                .json(serde_json::json!({ "email": email, "password": password })),
        )
        .await
    }

    /// Registers `email` and logs in, returning the session token.
    pub async fn signed_in(&self, email: &str) -> String {
        assert_eq!(
            self.register(email, PASSWORD).await.status,
            StatusCode::CREATED
        );
        let login = self.login(email, PASSWORD).await;
        assert_eq!(login.status, StatusCode::OK, "body: {}", login.body);
        login.session_token().expect("login sets session cookie")
    }

    pub async fn csrf_token(&self, session: &str) -> String {
        let response = self
            .send(Req::get("/api/v1/auth/csrf").session(session))
            .await;
        assert_eq!(response.status, StatusCode::OK);
        response.body["csrf_token"].as_str().unwrap().to_owned()
    }

    pub async fn me(&self, session: &str) -> TestResponse {
        self.send(Req::get("/api/v1/auth/me").session(session))
            .await
    }
}
