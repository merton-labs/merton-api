//! HTTP routing and OpenAPI description.

mod auth;
mod error;
pub mod extract;
mod health;
mod portfolios;

use axum::http::{HeaderValue, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::app::AppState;

#[derive(OpenApi)]
#[openapi(
    info(title = "Merton API"),
    paths(
        health::health,
        auth::register,
        auth::login,
        auth::logout,
        auth::me,
        auth::csrf,
        portfolios::create,
        portfolios::list,
        portfolios::get,
        portfolios::update,
        portfolios::archive,
        portfolios::restore,
        portfolios::delete,
    ),
    modifiers(&SessionCookieScheme),
    tags(
        (name = "health", description = "Service health"),
        (name = "auth", description = "Registration, login and sessions. Errors use RFC 9457 Problem Details."),
        (name = "portfolios", description = "The current user's private portfolios. Other users' portfolios are reported as 404."),
    ),
)]
pub struct ApiDoc;

struct SessionCookieScheme;

impl Modify for SessionCookieScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session_cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                extract::SECURE_SESSION_COOKIE,
                "HttpOnly session cookie set by login. Named `merton_session` when \
                 SESSION_COOKIE_SECURE=false (localhost development only).",
            ))),
        );
    }
}

pub fn router(state: AppState) -> Router {
    let auth = Router::new()
        .route("/register", post(auth::register))
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/csrf", get(auth::csrf))
        .layer(no_store());

    let portfolios = Router::new()
        .route("/", get(portfolios::list).post(portfolios::create))
        .route(
            "/{id}",
            get(portfolios::get)
                .patch(portfolios::update)
                .delete(portfolios::delete),
        )
        .route("/{id}/archive", post(portfolios::archive))
        .route("/{id}/restore", post(portfolios::restore))
        .layer(no_store());

    let api_v1 = Router::new()
        .route("/health", get(health::health))
        .route("/openapi.json", get(openapi_json))
        .nest("/auth", auth)
        .nest("/portfolios", portfolios);

    Router::new()
        .nest("/api/v1", api_v1)
        .fallback(error::not_found)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state)
}

/// Authentication and private financial data must never be cached.
fn no_store() -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
