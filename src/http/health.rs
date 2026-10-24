use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use utoipa::ToSchema;

use crate::app::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: HealthStatus,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Ok,
    Unavailable,
}

/// Reports whether the service and its database are reachable.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    tag = "health",
    responses(
        (status = OK, description = "Service is healthy", body = HealthResponse),
        (status = SERVICE_UNAVAILABLE, description = "Database is unreachable", body = HealthResponse),
    ),
)]
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    match state.db.ping().await {
        Ok(()) => (
            StatusCode::OK,
            Json(HealthResponse {
                status: HealthStatus::Ok,
            }),
        ),
        Err(err) => {
            tracing::warn!(error = %err, "database health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: HealthStatus::Unavailable,
                }),
            )
        }
    }
}
