//! Portfolio endpoints. Reads need a session; mutations also need CSRF.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{
    ApiJson, ApiQuery, CsrfProtectedSession, CurrentSession, ResourceId, reject_null,
};
use crate::app::AppState;
use crate::pagination::Page;
use crate::portfolios::service::{self, PortfolioChanges};
use crate::portfolios::{Portfolio, PortfolioError, PortfolioList, PortfolioStatus};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioStatusDto {
    Active,
    Archived,
}

impl From<PortfolioStatus> for PortfolioStatusDto {
    fn from(status: PortfolioStatus) -> Self {
        match status {
            PortfolioStatus::Active => Self::Active,
            PortfolioStatus::Archived => Self::Archived,
        }
    }
}

impl From<PortfolioStatusDto> for PortfolioStatus {
    fn from(status: PortfolioStatusDto) -> Self {
        match status {
            PortfolioStatusDto::Active => Self::Active,
            PortfolioStatusDto::Archived => Self::Archived,
        }
    }
}

/// The owner is always the authenticated user; owner fields are rejected.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePortfolioRequest {
    #[schema(example = "Retirement", min_length = 1, max_length = 100)]
    pub name: String,
    /// Three-letter currency code used for reporting.
    #[schema(example = "THB", pattern = "^[A-Za-z]{3}$")]
    pub base_currency: String,
}

/// Send at least one field. Changing `base_currency` never alters recorded
/// transactions; valuations are recalculated on demand.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePortfolioRequest {
    #[serde(default, deserialize_with = "reject_null")]
    #[schema(example = "Long-term", min_length = 1, max_length = 100)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "reject_null")]
    #[schema(example = "USD", pattern = "^[A-Za-z]{3}$")]
    pub base_currency: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortfolioResponse {
    pub id: Uuid,
    #[schema(example = "Retirement")]
    pub name: String,
    #[schema(example = "THB")]
    pub base_currency: String,
    pub status: PortfolioStatusDto,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

impl From<Portfolio> for PortfolioResponse {
    fn from(p: Portfolio) -> Self {
        Self {
            id: p.id,
            name: p.name,
            base_currency: p.base_currency,
            status: p.status.into(),
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortfolioListResponse {
    pub items: Vec<PortfolioResponse>,
    /// Number of matching portfolios across all pages.
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

impl From<PortfolioList> for PortfolioListResponse {
    fn from(list: PortfolioList) -> Self {
        Self {
            items: list.items.into_iter().map(Into::into).collect(),
            total: list.total,
            limit: list.page.limit,
            offset: list.page.offset,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListPortfoliosQuery {
    /// Page size (default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u64>,
    /// Number of portfolios to skip (default 0).
    #[param(minimum = 0)]
    pub offset: Option<u64>,
    /// Only portfolios with this status; all statuses when omitted.
    pub status: Option<PortfolioStatusDto>,
}

/// Create a portfolio owned by the current user.
#[utoipa::path(
    post,
    path = "/api/v1/portfolios",
    tag = "portfolios",
    security(("session_cookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`")),
    request_body = CreatePortfolioRequest,
    responses(
        (status = CREATED, description = "Portfolio created", body = PortfolioResponse,
            headers(("Location" = String, description = "URL of the new portfolio"))),
        (status = BAD_REQUEST, description = "Malformed body or unknown field", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid name or base currency", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ApiJson(body): ApiJson<CreatePortfolioRequest>,
) -> Result<
    (
        StatusCode,
        [(header::HeaderName, String); 1],
        Json<PortfolioResponse>,
    ),
    ApiError,
> {
    let portfolio =
        service::create(&state.db, session.user.id, &body.name, &body.base_currency).await?;
    let location = format!("/api/v1/portfolios/{}", portfolio.id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(portfolio.into()),
    ))
}

/// List the current user's portfolios in creation order.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios",
    tag = "portfolios",
    security(("session_cookie" = [])),
    params(ListPortfoliosQuery),
    responses(
        (status = OK, description = "One page of the user's portfolios", body = PortfolioListResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Page size or offset out of range", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ApiQuery(query): ApiQuery<ListPortfoliosQuery>,
) -> Result<Json<PortfolioListResponse>, ApiError> {
    let page = Page::new(query.limit, query.offset)
        .map_err(|field| PortfolioError::Validation(vec![field]))?;
    let list = service::list(
        &state.db,
        session.user.id,
        query.status.map(Into::into),
        page,
    )
    .await?;
    Ok(Json(list.into()))
}

/// Get one of the current user's portfolios.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}",
    tag = "portfolios",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID")),
    responses(
        (status = OK, description = "The portfolio", body = PortfolioResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn get(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(id): ResourceId,
) -> Result<Json<PortfolioResponse>, ApiError> {
    let portfolio = service::get(&state.db, session.user.id, id).await?;
    Ok(Json(portfolio.into()))
}

/// Rename an active portfolio or change its base currency.
#[utoipa::path(
    patch,
    path = "/api/v1/portfolios/{id}",
    tag = "portfolios",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = UpdatePortfolioRequest,
    responses(
        (status = OK, description = "Updated portfolio", body = PortfolioResponse),
        (status = BAD_REQUEST, description = "Malformed body or unknown field", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Portfolio is archived (`portfolio_archived`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid or missing fields", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn update(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(id): ResourceId,
    ApiJson(body): ApiJson<UpdatePortfolioRequest>,
) -> Result<Json<PortfolioResponse>, ApiError> {
    let changes = PortfolioChanges {
        name: body.name.as_deref(),
        base_currency: body.base_currency.as_deref(),
    };
    let portfolio = service::update(&state.db, session.user.id, id, changes).await?;
    Ok(Json(portfolio.into()))
}

/// Archive an active portfolio. Archived portfolios stay readable.
#[utoipa::path(
    post,
    path = "/api/v1/portfolios/{id}/archive",
    tag = "portfolios",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = OK, description = "Archived portfolio", body = PortfolioResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Already archived (`portfolio_archived`)", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn archive(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(id): ResourceId,
) -> Result<Json<PortfolioResponse>, ApiError> {
    let portfolio = service::archive(&state.db, session.user.id, id).await?;
    Ok(Json(portfolio.into()))
}

/// Restore an archived portfolio to active.
#[utoipa::path(
    post,
    path = "/api/v1/portfolios/{id}/restore",
    tag = "portfolios",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = OK, description = "Restored portfolio", body = PortfolioResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Not archived (`portfolio_not_archived`)", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn restore(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(id): ResourceId,
) -> Result<Json<PortfolioResponse>, ApiError> {
    let portfolio = service::restore(&state.db, session.user.id, id).await?;
    Ok(Json(portfolio.into()))
}

/// Permanently delete a portfolio that has no financial history.
#[utoipa::path(
    delete,
    path = "/api/v1/portfolios/{id}",
    tag = "portfolios",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = NO_CONTENT, description = "Portfolio deleted"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Has financial history (`portfolio_has_history`)", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn delete(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(id): ResourceId,
) -> Result<StatusCode, ApiError> {
    service::delete(&state.db, session.user.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
