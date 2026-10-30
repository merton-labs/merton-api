//! Holdings, valuation and allocation at an explicit `as_of` date. All are
//! computed on request and nothing is stored (ADR-VAL-01). Reads work on
//! archived portfolios too.

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use time::Date;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{ApiQuery, CurrentSession, ResourceId};
use super::instruments::AssetClassDto;
use super::market_data::SourceDto;
use super::transactions::iso_date;
use crate::app::AppState;
use crate::valuation::{
    Allocation, AllocationClass, CashBalance, FxQuote, Holdings, Position, PriceQuote,
    UnvaluedReason, Valuation, service,
};

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AsOfQuery {
    /// Required valuation date (`YYYY-MM-DD`). Only data dated on or before
    /// it is used.
    pub as_of: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PositionResponse {
    pub instrument_id: Uuid,
    pub name: String,
    pub symbol: Option<String>,
    pub asset_class: AssetClassDto,
    pub quote_currency: String,
    /// Decimal string.
    #[schema(example = "100")]
    pub quantity: String,
    /// Cost of the open FIFO lots in the quote currency (decimal string).
    #[schema(example = "3525.00")]
    pub cost_basis: String,
}

impl From<Position> for PositionResponse {
    fn from(p: Position) -> Self {
        Self {
            instrument_id: p.instrument_id,
            name: p.name,
            symbol: p.symbol,
            asset_class: p.asset_class.into(),
            quote_currency: p.quote_currency,
            quantity: p.quantity.to_string(),
            cost_basis: p.cost_basis.to_string(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CashBalanceResponse {
    pub cash_account_id: Uuid,
    pub name: String,
    pub currency: String,
    /// Signed decimal string; cash may be negative.
    #[schema(example = "-250.00")]
    pub balance: String,
}

impl From<CashBalance> for CashBalanceResponse {
    fn from(c: CashBalance) -> Self {
        Self {
            cash_account_id: c.cash_account_id,
            name: c.name,
            currency: c.currency,
            balance: c.balance.to_string(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HoldingsResponse {
    pub portfolio_id: Uuid,
    #[schema(value_type = String, format = Date, example = "2026-01-15")]
    #[serde(serialize_with = "iso_date")]
    pub as_of: Date,
    pub base_currency: String,
    /// Open positions, by name.
    pub positions: Vec<PositionResponse>,
    /// Every cash account of the portfolio, in creation order.
    pub cash: Vec<CashBalanceResponse>,
}

impl From<Holdings> for HoldingsResponse {
    fn from(h: Holdings) -> Self {
        Self {
            portfolio_id: h.portfolio_id,
            as_of: h.as_of,
            base_currency: h.base_currency,
            positions: h.positions.into_iter().map(Into::into).collect(),
            cash: h.cash.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PriceQuoteResponse {
    #[schema(example = "36.50")]
    pub price: String,
    #[schema(value_type = String, format = Date)]
    #[serde(serialize_with = "iso_date")]
    pub date: Date,
    pub source: SourceDto,
    /// More than 7 days before `as_of`. Stale prices are still used.
    pub stale: bool,
}

impl From<PriceQuote> for PriceQuoteResponse {
    fn from(q: PriceQuote) -> Self {
        Self {
            price: q.price.to_string(),
            date: q.date,
            source: q.source.into(),
            stale: q.stale,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FxQuoteResponse {
    /// The stored rate, as entered.
    #[schema(example = "36.25")]
    pub rate: String,
    #[schema(value_type = String, format = Date)]
    #[serde(serialize_with = "iso_date")]
    pub date: Date,
    pub source: SourceDto,
    /// The stored pair is base/currency, so the value was divided by `rate`.
    pub inverse: bool,
    /// More than 3 days before `as_of`. Stale rates are still used.
    pub stale: bool,
}

impl From<FxQuote> for FxQuoteResponse {
    fn from(q: FxQuote) -> Self {
        Self {
            rate: q.rate.to_string(),
            date: q.date,
            source: q.source.into(),
            inverse: q.inverse,
            stale: q.stale,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnvaluedReasonDto {
    MissingPrice,
    MissingFxRate,
    /// The exact value is too large to represent.
    OutOfRange,
}

impl From<UnvaluedReason> for UnvaluedReasonDto {
    fn from(reason: UnvaluedReason) -> Self {
        match reason {
            UnvaluedReason::MissingPrice => Self::MissingPrice,
            UnvaluedReason::MissingFxRate => Self::MissingFxRate,
            UnvaluedReason::OutOfRange => Self::OutOfRange,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValuedPositionResponse {
    #[serde(flatten)]
    pub position: PositionResponse,
    /// The latest price on or before `as_of`, if any.
    pub price: Option<PriceQuoteResponse>,
    /// quantity × price in the quote currency, exact.
    pub market_value: Option<String>,
    /// The rate used; absent when the quote currency is the base currency.
    pub fx: Option<FxQuoteResponse>,
    /// In the base currency, rounded half-to-even to 8 places.
    pub value: Option<String>,
    /// Why `value` is missing.
    pub unvalued_reason: Option<UnvaluedReasonDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValuedCashResponse {
    #[serde(flatten)]
    pub cash: CashBalanceResponse,
    pub fx: Option<FxQuoteResponse>,
    /// In the base currency, rounded half-to-even to 8 places.
    pub value: Option<String>,
    pub unvalued_reason: Option<UnvaluedReasonDto>,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssetKindDto {
    Position,
    Cash,
}

/// An asset left out of the totals (ADR-VAL-04).
#[derive(Debug, Serialize, ToSchema)]
pub struct UnvaluedAssetResponse {
    pub kind: AssetKindDto,
    /// The instrument ID for a position, the cash account ID for cash.
    pub id: Uuid,
    pub reason: UnvaluedReasonDto,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValuationResponse {
    pub portfolio_id: Uuid,
    #[schema(value_type = String, format = Date, example = "2026-01-15")]
    #[serde(serialize_with = "iso_date")]
    pub as_of: Date,
    pub base_currency: String,
    /// Some asset could not be valued; the totals leave it out.
    pub partial: bool,
    /// Sum of the valued positions in the base currency.
    pub positions_value: String,
    /// Sum of the valued cash balances in the base currency; may be negative.
    pub cash_value: String,
    /// Net Portfolio Value: positions plus cash.
    pub net_value: String,
    pub positions: Vec<ValuedPositionResponse>,
    pub cash: Vec<ValuedCashResponse>,
    pub unvalued: Vec<UnvaluedAssetResponse>,
}

impl From<Valuation> for ValuationResponse {
    fn from(v: Valuation) -> Self {
        let mut unvalued = Vec::new();
        let positions = v
            .positions
            .into_iter()
            .map(|p| {
                if let Some(reason) = p.unvalued {
                    unvalued.push(UnvaluedAssetResponse {
                        kind: AssetKindDto::Position,
                        id: p.position.instrument_id,
                        reason: reason.into(),
                    });
                }
                ValuedPositionResponse {
                    position: p.position.into(),
                    price: p.price.map(Into::into),
                    market_value: p.market_value.map(|m| m.to_string()),
                    fx: p.fx.map(Into::into),
                    value: p.value.map(|m| m.to_string()),
                    unvalued_reason: p.unvalued.map(Into::into),
                }
            })
            .collect();
        let cash = v
            .cash
            .into_iter()
            .map(|c| {
                if let Some(reason) = c.unvalued {
                    unvalued.push(UnvaluedAssetResponse {
                        kind: AssetKindDto::Cash,
                        id: c.cash.cash_account_id,
                        reason: reason.into(),
                    });
                }
                ValuedCashResponse {
                    cash: c.cash.into(),
                    fx: c.fx.map(Into::into),
                    value: c.value.map(|m| m.to_string()),
                    unvalued_reason: c.unvalued.map(Into::into),
                }
            })
            .collect();
        Self {
            portfolio_id: v.portfolio_id,
            as_of: v.as_of,
            base_currency: v.base_currency,
            partial: v.partial,
            positions_value: v.positions_value.to_string(),
            cash_value: v.cash_value.to_string(),
            net_value: v.net_value.to_string(),
            positions,
            cash,
            unvalued,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AllocationClassDto {
    Stock,
    Etf,
    MutualFund,
    Cash,
}

impl From<AllocationClass> for AllocationClassDto {
    fn from(class: AllocationClass) -> Self {
        match class {
            AllocationClass::Stock => Self::Stock,
            AllocationClass::Etf => Self::Etf,
            AllocationClass::MutualFund => Self::MutualFund,
            AllocationClass::Cash => Self::Cash,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ClassShareResponse {
    pub asset_class: AllocationClassDto,
    /// In the base currency; cash may be negative.
    pub value: String,
    /// Signed percentage of the Net Portfolio Value, 4 places; `null` when
    /// the Net Portfolio Value is zero or negative.
    #[schema(example = "125.0000")]
    pub percent: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AllocationResponse {
    pub portfolio_id: Uuid,
    #[schema(value_type = String, format = Date, example = "2026-01-15")]
    #[serde(serialize_with = "iso_date")]
    pub as_of: Date,
    pub base_currency: String,
    /// Some asset could not be valued, so the allocation is incomplete.
    pub partial: bool,
    /// A cash account is overdrawn. This is not treated as a margin loan.
    pub negative_cash: bool,
    pub net_value: String,
    /// Asset classes with at least one valued asset.
    pub classes: Vec<ClassShareResponse>,
}

impl From<Allocation> for AllocationResponse {
    fn from(a: Allocation) -> Self {
        Self {
            portfolio_id: a.portfolio_id,
            as_of: a.as_of,
            base_currency: a.base_currency,
            partial: a.partial,
            negative_cash: a.negative_cash,
            net_value: a.net_value.to_string(),
            classes: a
                .classes
                .into_iter()
                .map(|c| ClassShareResponse {
                    asset_class: c.class.into(),
                    value: c.value.to_string(),
                    percent: c.percent.map(|p| p.to_string()),
                })
                .collect(),
        }
    }
}

/// Open positions and cash balances at `as_of`.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/holdings",
    tag = "valuation",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID"), AsOfQuery),
    responses(
        (status = OK, description = "Holdings at `as_of`", body = HoldingsResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Missing or invalid `as_of`", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn holdings(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(portfolio_id): ResourceId,
    ApiQuery(query): ApiQuery<AsOfQuery>,
) -> Result<Json<HoldingsResponse>, ApiError> {
    let holdings = service::holdings(
        &state.db,
        session.user.id,
        portfolio_id,
        query.as_of.as_deref(),
    )
    .await?;
    Ok(Json(holdings.into()))
}

/// Value the portfolio at `as_of` in its base currency.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/valuation",
    tag = "valuation",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID"), AsOfQuery),
    responses(
        (status = OK, description = "Valuation at `as_of`; check `partial`", body = ValuationResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Missing or invalid `as_of`, or totals too large", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn valuation(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(portfolio_id): ResourceId,
    ApiQuery(query): ApiQuery<AsOfQuery>,
) -> Result<Json<ValuationResponse>, ApiError> {
    let valuation = service::valuation(
        &state.db,
        session.user.id,
        portfolio_id,
        query.as_of.as_deref(),
    )
    .await?;
    Ok(Json(valuation.into()))
}

/// Signed allocation by asset class on Net Portfolio Value at `as_of`.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/allocation",
    tag = "valuation",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID"), AsOfQuery),
    responses(
        (status = OK, description = "Allocation at `as_of`; check `partial`", body = AllocationResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Missing or invalid `as_of`, or totals too large", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn allocation(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(portfolio_id): ResourceId,
    ApiQuery(query): ApiQuery<AsOfQuery>,
) -> Result<Json<AllocationResponse>, ApiError> {
    let allocation = service::allocation(
        &state.db,
        session.user.id,
        portfolio_id,
        query.as_of.as_deref(),
    )
    .await?;
    Ok(Json(allocation.into()))
}
