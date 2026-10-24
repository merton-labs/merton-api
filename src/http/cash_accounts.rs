//! Cash account endpoints, nested under a portfolio. Reads work on archived
//! portfolios; changes require the portfolio to be active.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{
    ApiJson, ApiQuery, CsrfProtectedSession, CurrentSession, ResourceId, ResourceIds,
};
use crate::app::AppState;
use crate::cash_accounts::service;
use crate::cash_accounts::{CashAccount, CashAccountError, CashAccountList};
use crate::pagination::Page;

/// Several accounts may share a currency.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCashAccountRequest {
    #[schema(example = "Broker THB", min_length = 1, max_length = 100)]
    pub name: String,
    /// Three-letter currency code. Cannot be changed later.
    #[schema(example = "THB", pattern = "^[A-Za-z]{3}$")]
    pub currency: String,
}

/// Only the name can change; the currency is fixed.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCashAccountRequest {
    #[schema(example = "Savings", min_length = 1, max_length = 100)]
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CashAccountResponse {
    pub id: Uuid,
    pub portfolio_id: Uuid,
    pub name: String,
    pub currency: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

impl From<CashAccount> for CashAccountResponse {
    fn from(a: CashAccount) -> Self {
        Self {
            id: a.id,
            portfolio_id: a.portfolio_id,
            name: a.name,
            currency: a.currency,
            created_at: a.created_at,
            updated_at: a.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CashAccountListResponse {
    pub items: Vec<CashAccountResponse>,
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

impl From<CashAccountList> for CashAccountListResponse {
    fn from(list: CashAccountList) -> Self {
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
pub struct ListCashAccountsQuery {
    /// Page size (default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u64>,
    /// Number of accounts to skip (default 0).
    #[param(minimum = 0)]
    pub offset: Option<u64>,
}

/// Add a cash account to an active portfolio.
#[utoipa::path(
    post,
    path = "/api/v1/portfolios/{id}/cash-accounts",
    tag = "cash-accounts",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = CreateCashAccountRequest,
    responses(
        (status = CREATED, description = "Cash account created", body = CashAccountResponse,
            headers(("Location" = String, description = "URL of the new cash account"))),
        (status = BAD_REQUEST, description = "Malformed body or unknown field", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Portfolio is archived (`portfolio_archived`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid name or currency", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(portfolio_id): ResourceId,
    ApiJson(body): ApiJson<CreateCashAccountRequest>,
) -> Result<
    (
        StatusCode,
        [(header::HeaderName, String); 1],
        Json<CashAccountResponse>,
    ),
    ApiError,
> {
    let account = service::create(
        &state.db,
        session.user.id,
        portfolio_id,
        &body.name,
        &body.currency,
    )
    .await?;
    let location = format!(
        "/api/v1/portfolios/{}/cash-accounts/{}",
        account.portfolio_id, account.id
    );
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(account.into()),
    ))
}

/// List a portfolio's cash accounts in creation order.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/cash-accounts",
    tag = "cash-accounts",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID"), ListCashAccountsQuery),
    responses(
        (status = OK, description = "One page of the portfolio's cash accounts", body = CashAccountListResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Page size or offset out of range", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(portfolio_id): ResourceId,
    ApiQuery(query): ApiQuery<ListCashAccountsQuery>,
) -> Result<Json<CashAccountListResponse>, ApiError> {
    let page = Page::new(query.limit, query.offset)
        .map_err(|field| CashAccountError::Validation(vec![field]))?;
    let list = service::list(&state.db, session.user.id, portfolio_id, page).await?;
    Ok(Json(list.into()))
}

/// Get one cash account.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/cash-accounts/{account_id}",
    tag = "cash-accounts",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("account_id" = Uuid, Path, description = "Cash account ID"),
    ),
    responses(
        (status = OK, description = "The cash account", body = CashAccountResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio or account for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn get(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceIds(portfolio_id, id): ResourceIds,
) -> Result<Json<CashAccountResponse>, ApiError> {
    let account = service::get(&state.db, session.user.id, portfolio_id, id).await?;
    Ok(Json(account.into()))
}

/// Rename a cash account in an active portfolio.
#[utoipa::path(
    patch,
    path = "/api/v1/portfolios/{id}/cash-accounts/{account_id}",
    tag = "cash-accounts",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("account_id" = Uuid, Path, description = "Cash account ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = UpdateCashAccountRequest,
    responses(
        (status = OK, description = "Updated cash account", body = CashAccountResponse),
        (status = BAD_REQUEST, description = "Malformed body or unknown field (including `currency`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio or account for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Portfolio is archived (`portfolio_archived`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid name", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn update(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceIds(portfolio_id, id): ResourceIds,
    ApiJson(body): ApiJson<UpdateCashAccountRequest>,
) -> Result<Json<CashAccountResponse>, ApiError> {
    let account = service::rename(&state.db, session.user.id, portfolio_id, id, &body.name).await?;
    Ok(Json(account.into()))
}

/// Delete a cash account that has no recorded movements.
#[utoipa::path(
    delete,
    path = "/api/v1/portfolios/{id}/cash-accounts/{account_id}",
    tag = "cash-accounts",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("account_id" = Uuid, Path, description = "Cash account ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = NO_CONTENT, description = "Cash account deleted"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio or account for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Portfolio archived (`portfolio_archived`) or account in use (`cash_account_in_use`)", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn delete(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceIds(portfolio_id, id): ResourceIds,
) -> Result<StatusCode, ApiError> {
    service::delete(&state.db, session.user.id, portfolio_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
