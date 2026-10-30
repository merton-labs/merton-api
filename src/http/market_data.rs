//! Market data endpoints: instrument prices, nested under an instrument, and
//! the user's FX rates. Reads need a session; mutations also need CSRF.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{
    ApiJson, ApiQuery, CsrfProtectedSession, CurrentSession, ResourceId, ResourceIds,
};
use super::transactions::iso_date;
use crate::app::AppState;
use crate::market_data::service::{self, DateQuery, FxRateInput, FxRateQuery, PriceInput};
use crate::market_data::{FxRate, FxRateList, InstrumentPrice, MarketDataError, PriceList, Source};
use crate::pagination::Page;

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceDto {
    /// Entered through the API.
    Manual,
    /// Imported from CSV.
    Import,
}

impl From<Source> for SourceDto {
    fn from(source: Source) -> Self {
        match source {
            Source::Manual => Self::Manual,
            Source::Import => Self::Import,
        }
    }
}

// --- Prices ------------------------------------------------------------------

/// A closing price in the instrument's quote currency. One per date.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PriceRequest {
    #[schema(example = "2026-01-15", format = Date)]
    pub price_date: String,
    #[schema(example = "35.25", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub price: String,
}

impl PriceRequest {
    fn input(&self) -> PriceInput<'_> {
        PriceInput {
            price_date: &self.price_date,
            price: &self.price,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PriceResponse {
    pub id: Uuid,
    pub instrument_id: Uuid,
    #[schema(value_type = String, format = Date, example = "2026-01-15")]
    #[serde(serialize_with = "iso_date")]
    pub price_date: Date,
    /// Decimal string in the instrument's quote currency.
    #[schema(example = "35.25")]
    pub price: String,
    pub source: SourceDto,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

impl From<InstrumentPrice> for PriceResponse {
    fn from(p: InstrumentPrice) -> Self {
        Self {
            id: p.id,
            instrument_id: p.instrument_id,
            price_date: p.price_date,
            price: p.price.to_string(),
            source: p.source.into(),
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PriceListResponse {
    pub items: Vec<PriceResponse>,
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

impl From<PriceList> for PriceListResponse {
    fn from(list: PriceList) -> Self {
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
pub struct ListPricesQuery {
    /// Page size (default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u64>,
    /// Number of prices to skip (default 0).
    #[param(minimum = 0)]
    pub offset: Option<u64>,
    /// Only prices on or after this date (`YYYY-MM-DD`).
    pub from: Option<String>,
    /// Only prices on or before this date (`YYYY-MM-DD`).
    pub to: Option<String>,
}

/// Record a price for one of the current user's instruments.
#[utoipa::path(
    post,
    path = "/api/v1/instruments/{id}/prices",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Instrument ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = PriceRequest,
    responses(
        (status = CREATED, description = "Price recorded", body = PriceResponse,
            headers(("Location" = String, description = "URL of the new price"))),
        (status = BAD_REQUEST, description = "Malformed body or unknown field", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "The instrument already has a price on this date (`price_exists`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid date or price", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn create_price(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(instrument_id): ResourceId,
    ApiJson(body): ApiJson<PriceRequest>,
) -> Result<
    (
        StatusCode,
        [(header::HeaderName, String); 1],
        Json<PriceResponse>,
    ),
    ApiError,
> {
    let price =
        service::create_price(&state.db, session.user.id, instrument_id, body.input()).await?;
    let location = format!("/api/v1/instruments/{instrument_id}/prices/{}", price.id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(price.into()),
    ))
}

/// List an instrument's prices, latest first.
#[utoipa::path(
    get,
    path = "/api/v1/instruments/{id}/prices",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Instrument ID"), ListPricesQuery),
    responses(
        (status = OK, description = "One page of the instrument's prices", body = PriceListResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid dates, page size or offset", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn list_prices(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(instrument_id): ResourceId,
    ApiQuery(query): ApiQuery<ListPricesQuery>,
) -> Result<Json<PriceListResponse>, ApiError> {
    let page = Page::new(query.limit, query.offset)
        .map_err(|field| MarketDataError::Validation(vec![field]))?;
    let dates = DateQuery {
        from: query.from.as_deref(),
        to: query.to.as_deref(),
    };
    let list = service::list_prices(&state.db, session.user.id, instrument_id, dates, page).await?;
    Ok(Json(list.into()))
}

/// Get one price of one of the current user's instruments.
#[utoipa::path(
    get,
    path = "/api/v1/instruments/{id}/prices/{price_id}",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Instrument ID"),
        ("price_id" = Uuid, Path, description = "Price ID"),
    ),
    responses(
        (status = OK, description = "The price", body = PriceResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument or price for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn get_price(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceIds(instrument_id, id): ResourceIds,
) -> Result<Json<PriceResponse>, ApiError> {
    let price = service::get_price(&state.db, session.user.id, instrument_id, id).await?;
    Ok(Json(price.into()))
}

/// Replace a price's date and value.
#[utoipa::path(
    put,
    path = "/api/v1/instruments/{id}/prices/{price_id}",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Instrument ID"),
        ("price_id" = Uuid, Path, description = "Price ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = PriceRequest,
    responses(
        (status = OK, description = "Updated price", body = PriceResponse),
        (status = BAD_REQUEST, description = "Malformed body or unknown field", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument or price for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Another price already has this date (`price_exists`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid date or price", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn replace_price(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceIds(instrument_id, id): ResourceIds,
    ApiJson(body): ApiJson<PriceRequest>,
) -> Result<Json<PriceResponse>, ApiError> {
    let price =
        service::replace_price(&state.db, session.user.id, instrument_id, id, body.input()).await?;
    Ok(Json(price.into()))
}

/// Delete a price.
#[utoipa::path(
    delete,
    path = "/api/v1/instruments/{id}/prices/{price_id}",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Instrument ID"),
        ("price_id" = Uuid, Path, description = "Price ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = NO_CONTENT, description = "Price deleted"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such instrument or price for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn delete_price(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceIds(instrument_id, id): ResourceIds,
) -> Result<StatusCode, ApiError> {
    service::delete_price(&state.db, session.user.id, instrument_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- FX rates ----------------------------------------------------------------

/// A market FX rate: one `base_currency` is worth `rate` `quote_currency`
/// on `rate_date`. One per pair and date. Never derived from `FX_EXCHANGE`
/// transactions.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FxRateRequest {
    #[schema(example = "USD", pattern = "^[A-Za-z]{3}$")]
    pub base_currency: String,
    /// Must differ from `base_currency`.
    #[schema(example = "THB", pattern = "^[A-Za-z]{3}$")]
    pub quote_currency: String,
    #[schema(example = "2026-01-15", format = Date)]
    pub rate_date: String,
    /// Greater than zero.
    #[schema(example = "36.25", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub rate: String,
}

impl FxRateRequest {
    fn input(&self) -> FxRateInput<'_> {
        FxRateInput {
            base_currency: &self.base_currency,
            quote_currency: &self.quote_currency,
            rate_date: &self.rate_date,
            rate: &self.rate,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FxRateResponse {
    pub id: Uuid,
    pub base_currency: String,
    pub quote_currency: String,
    #[schema(value_type = String, format = Date, example = "2026-01-15")]
    #[serde(serialize_with = "iso_date")]
    pub rate_date: Date,
    /// Decimal string: units of `quote_currency` per one `base_currency`.
    #[schema(example = "36.25")]
    pub rate: String,
    pub source: SourceDto,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

impl From<FxRate> for FxRateResponse {
    fn from(r: FxRate) -> Self {
        Self {
            id: r.id,
            base_currency: r.base_currency,
            quote_currency: r.quote_currency,
            rate_date: r.rate_date,
            rate: r.rate.to_string(),
            source: r.source.into(),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FxRateListResponse {
    pub items: Vec<FxRateResponse>,
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

impl From<FxRateList> for FxRateListResponse {
    fn from(list: FxRateList) -> Self {
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
pub struct ListFxRatesQuery {
    /// Page size (default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u64>,
    /// Number of rates to skip (default 0).
    #[param(minimum = 0)]
    pub offset: Option<u64>,
    /// Only rates with this base currency.
    pub base_currency: Option<String>,
    /// Only rates with this quote currency.
    pub quote_currency: Option<String>,
    /// Only rates on or after this date (`YYYY-MM-DD`).
    pub from: Option<String>,
    /// Only rates on or before this date (`YYYY-MM-DD`).
    pub to: Option<String>,
}

/// Record an FX rate for the current user.
#[utoipa::path(
    post,
    path = "/api/v1/fx-rates",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`")),
    request_body = FxRateRequest,
    responses(
        (status = CREATED, description = "Rate recorded", body = FxRateResponse,
            headers(("Location" = String, description = "URL of the new rate"))),
        (status = BAD_REQUEST, description = "Malformed body or unknown field", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "A rate for this pair already exists on this date (`fx_rate_exists`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid currencies, date or rate", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn create_fx_rate(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ApiJson(body): ApiJson<FxRateRequest>,
) -> Result<
    (
        StatusCode,
        [(header::HeaderName, String); 1],
        Json<FxRateResponse>,
    ),
    ApiError,
> {
    let rate = service::create_fx_rate(&state.db, session.user.id, body.input()).await?;
    let location = format!("/api/v1/fx-rates/{}", rate.id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(rate.into()),
    ))
}

/// List the current user's FX rates, latest first.
#[utoipa::path(
    get,
    path = "/api/v1/fx-rates",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(ListFxRatesQuery),
    responses(
        (status = OK, description = "One page of the user's FX rates", body = FxRateListResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid filters, page size or offset", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn list_fx_rates(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ApiQuery(query): ApiQuery<ListFxRatesQuery>,
) -> Result<Json<FxRateListResponse>, ApiError> {
    let page = Page::new(query.limit, query.offset)
        .map_err(|field| MarketDataError::Validation(vec![field]))?;
    let filter = FxRateQuery {
        base_currency: query.base_currency.as_deref(),
        quote_currency: query.quote_currency.as_deref(),
        dates: DateQuery {
            from: query.from.as_deref(),
            to: query.to.as_deref(),
        },
    };
    let list = service::list_fx_rates(&state.db, session.user.id, filter, page).await?;
    Ok(Json(list.into()))
}

/// Get one of the current user's FX rates.
#[utoipa::path(
    get,
    path = "/api/v1/fx-rates/{id}",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "FX rate ID")),
    responses(
        (status = OK, description = "The rate", body = FxRateResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such rate for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn get_fx_rate(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(id): ResourceId,
) -> Result<Json<FxRateResponse>, ApiError> {
    let rate = service::get_fx_rate(&state.db, session.user.id, id).await?;
    Ok(Json(rate.into()))
}

/// Replace an FX rate's pair, date and value.
#[utoipa::path(
    put,
    path = "/api/v1/fx-rates/{id}",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "FX rate ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = FxRateRequest,
    responses(
        (status = OK, description = "Updated rate", body = FxRateResponse),
        (status = BAD_REQUEST, description = "Malformed body or unknown field", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such rate for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Another rate already has this pair and date (`fx_rate_exists`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid currencies, date or rate", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn replace_fx_rate(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(id): ResourceId,
    ApiJson(body): ApiJson<FxRateRequest>,
) -> Result<Json<FxRateResponse>, ApiError> {
    let rate = service::replace_fx_rate(&state.db, session.user.id, id, body.input()).await?;
    Ok(Json(rate.into()))
}

/// Delete an FX rate.
#[utoipa::path(
    delete,
    path = "/api/v1/fx-rates/{id}",
    tag = "market-data",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "FX rate ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = NO_CONTENT, description = "Rate deleted"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such rate for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn delete_fx_rate(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(id): ResourceId,
) -> Result<StatusCode, ApiError> {
    service::delete_fx_rate(&state.db, session.user.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
