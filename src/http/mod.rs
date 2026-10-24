//! HTTP routing and OpenAPI description.

mod health;

use axum::routing::get;
use axum::{Json, Router};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;
use utoipa::OpenApi;

use crate::app::AppState;

#[derive(OpenApi)]
#[openapi(
    info(title = "Merton API"),
    paths(health::health),
    tags((name = "health", description = "Service health")),
)]
pub struct ApiDoc;

pub fn router(state: AppState) -> Router {
    let api_v1 = Router::new()
        .route("/health", get(health::health))
        .route("/openapi.json", get(openapi_json));

    Router::new()
        .nest("/api/v1", api_v1)
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
