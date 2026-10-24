//! Instrument endpoints. Reads need a session; mutations also need CSRF.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{
    ApiJson, ApiQuery, CsrfProtectedSession, CurrentSession, ResourceId, present, reject_null,
};
use crate::app::AppState;
use crate::instruments::service::{self, InstrumentChanges, NewInstrument};
use crate::instruments::{AssetClass, Instrument, InstrumentError, InstrumentList};
use crate::pagination::Page;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssetClassDto {
    Stock,
    Etf,
    MutualFund,
}

impl From<AssetClass> for AssetClassDto {
    fn from(class: AssetClass) -> Self {
        match class {
            AssetClass::Stock => Self::Stock,
            AssetClass::Etf => Self::Etf,
            AssetClass::MutualFund => Self::MutualFund,
        }
    }
}

impl From<AssetClassDto> for AssetClass {
    fn from(class: AssetClassDto) -> Self {
        match class {
            AssetClassDto::Stock => Self::Stock,
            AssetClassDto::Etf => Self::Etf,
            AssetClassDto::MutualFund => Self::MutualFund,
        }
    }
}

/// Identity fields are optional and need not be unique. The owner is always
/// the authenticated user.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInstrumentRequest {
    pub asset_class: AssetClassDto,
    #[schema(
        example = "PTT Public Company Limited",
        min_length = 1,
        max_length = 200
    )]
    pub name: String,
    #[schema(example = "PTT", max_length = 32)]
    pub symbol: Option<String>,
    /// ISO 6166 ISIN; the check digit is validated.
    #[schema(
        example = "US0378331005",
        min_length = 12,
        max_length = 12,
        pattern = "^[A-Za-z]{2}[A-Za-z0-9]{9}[0-9]$"
    )]
    pub isin: Option<String>,
    #[schema(example = "SET", max_length = 32)]
    pub exchange: Option<String>,
    /// Three-letter currency the instrument is priced in. Cannot be changed later.
    #[schema(example = "THB", pattern = "^[A-Za-z]{3}$")]
    pub quote_currency: String,
}

/// Send at least one field. `null` clears `symbol`, `isin` or `exchange`.
/// The quote currency cannot be changed.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInstrumentRequest {
    #[serde(default, deserialize_with = "reject_null")]
    pub asset_class: Option<AssetClassDto>,
    #[serde(default, deserialize_with = "reject_null")]
    #[schema(min_length = 1, max_length = 200)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable = true, max_length = 32)]
    pub symbol: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable = true, min_length = 12, max_length = 12, pattern = "^[A-Za-z]{2}[A-Za-z0-9]{9}[0-9]$")]
    pub isin: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable = true, max_length = 32)]
    pub exchange: Option<Option<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstrumentResponse {
    pub id: Uuid,
    pub asset_class: AssetClassDto,
    pub name: String,
    pub symbol: Option<String>,
    pub isin: Option<String>,
    pub exchange: Option<String>,
    pub quote_currency: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

impl From<Instrument> for InstrumentResponse {
    fn from(i: Instrument) -> Self {
        Self {
            id: i.id,
            asset_class: i.asset_class.into(),
            name: i.name,
            symbol: i.symbol,
            isin: i.isin,
            exchange: i.exchange,
            quote_currency: i.quote_currency,
            created_at: i.created_at,
            updated_at: i.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstrumentListResponse {
    pub items: Vec<InstrumentResponse>,
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

impl From<InstrumentList> for InstrumentListResponse {
    fn from(list: InstrumentList) -> Self {
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
pub struct ListInstrumentsQuery {
    /// Page size (default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u64>,
    /// Number of instruments to skip (default 0).
    #[param(minimum = 0)]
    pub offset: Option<u64>,
    /// Only instruments of this asset class.
    pub asset_class: Option<AssetClassDto>,
}

/// Create an instrument owned by the current user.
#[utoipa::path(
    post,
    path = "/api/v1/instruments",
    tag = "instruments",
    security(("session_cookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`")),
    request_body = CreateInstrumentRequest,
    responses(
        (status = CREATED, description = "Instrument created", body = InstrumentResponse,
            headers(("Location" = String, description = "URL of the new instrument"))),
        (status = BAD_REQUEST, description = "Malformed body, unknown field or unknown asset class", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid name, identifier or currency", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ApiJson(body): ApiJson<CreateInstrumentRequest>,
) -> Result<
    (
        StatusCode,
        [(header::HeaderName, String); 1],
        Json<InstrumentResponse>,
    ),
    ApiError,
> {
    let new = NewInstrument {
        asset_class: body.asset_class.into(),
        name: &body.name,
        symbol: body.symbol.as_deref(),
        isin: body.isin.as_deref(),
        exchange: body.exchange.as_deref(),
        quote_currency: &body.quote_currency,
    };
    let instrument = service::create(&state.db, session.user.id, new).await?;
    let location = format!("/api/v1/instruments/{}", instrument.id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(instrument.into()),
    ))
}

/// List the current user's instruments in creation order.
#[utoipa::path(
    get,
    path = "/api/v1/instruments",
    tag = "instruments",
    security(("session_cookie" = [])),
    params(ListInstrumentsQuery),
    responses(
        (status = OK, description = "One page of the user's instruments", body = InstrumentListResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Page size or offset out of range", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ApiQuery(query): ApiQuery<ListInstrumentsQuery>,
) -> Result<Json<InstrumentListResponse>, ApiError> {
    let page = Page::new(query.limit, query.offset)
        .map_err(|field| InstrumentError::Validation(vec![field]))?;
    let list = service::list(
        &state.db,
        session.user.id,
        query.asset_class.map(Into::into),
        page,
    )
    .await?;
    Ok(Json(list.into()))
}

/// Get one of the current user's instruments.
#[utoipa::path(
    get,
    path = "/api/v1/instruments/{id}",
    tag = "instruments",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Instrument ID")),
    responses(
        (status = OK, description = "The instrument", body = InstrumentResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn get(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(id): ResourceId,
) -> Result<Json<InstrumentResponse>, ApiError> {
    let instrument = service::get(&state.db, session.user.id, id).await?;
    Ok(Json(instrument.into()))
}

/// Update an instrument's descriptive attributes.
#[utoipa::path(
    patch,
    path = "/api/v1/instruments/{id}",
    tag = "instruments",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Instrument ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = UpdateInstrumentRequest,
    responses(
        (status = OK, description = "Updated instrument", body = InstrumentResponse),
        (status = BAD_REQUEST, description = "Malformed body or unknown field (including `quote_currency`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid or missing fields", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn update(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(id): ResourceId,
    ApiJson(body): ApiJson<UpdateInstrumentRequest>,
) -> Result<Json<InstrumentResponse>, ApiError> {
    let changes = InstrumentChanges {
        asset_class: body.asset_class.map(Into::into),
        name: body.name.as_deref(),
        symbol: body.symbol.as_ref().map(Option::as_deref),
        isin: body.isin.as_ref().map(Option::as_deref),
        exchange: body.exchange.as_ref().map(Option::as_deref),
    };
    let instrument = service::update(&state.db, session.user.id, id, changes).await?;
    Ok(Json(instrument.into()))
}

/// Delete an instrument that no other record refers to.
#[utoipa::path(
    delete,
    path = "/api/v1/instruments/{id}",
    tag = "instruments",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Instrument ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = NO_CONTENT, description = "Instrument deleted"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Referenced by other records (`instrument_in_use`)", body = ProblemDetails, content_type = "application/problem+json"),
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
