//! HTTP routing and OpenAPI description.

mod auth;
mod error;
pub mod extract;
mod health;

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
    ),
    modifiers(&SessionCookieScheme),
    tags(
        (name = "health", description = "Service health"),
        (name = "auth", description = "Registration, login and sessions. Errors use RFC 9457 Problem Details."),
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
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ));

    let api_v1 = Router::new()
        .route("/health", get(health::health))
        .route("/openapi.json", get(openapi_json))
        .nest("/auth", auth);

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

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
