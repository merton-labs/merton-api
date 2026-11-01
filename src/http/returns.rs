//! Returns per holding at an explicit `as_of` date (ADR-RET-01), computed
//! on request. Reads work on archived portfolios too.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use time::Date;
use utoipa::ToSchema;
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{ApiQuery, CurrentSession, ResourceId};
use super::instruments::AssetClassDto;
use super::transactions::iso_date;
use super::valuation::{AsOfQuery, PriceQuoteResponse, UnvaluedReasonDto};
use crate::app::AppState;
use crate::returns::{HoldingReturn, Returns, service};

#[derive(Debug, Serialize, ToSchema)]
pub struct HoldingReturnResponse {
    pub instrument_id: Uuid,
    pub name: String,
    pub symbol: Option<String>,
    pub asset_class: AssetClassDto,
    /// Every amount below is in this currency.
    pub quote_currency: String,
    /// Open quantity; `0` for a sold position.
    #[schema(example = "1.0655395")]
    pub quantity: String,
    /// Cost of the open FIFO lots, without fees or taxes.
    #[schema(example = "254.86")]
    pub cost_basis: String,
    /// Cost basis ÷ quantity, 8 places; `null` for a sold position.
    #[schema(example = "239.18400022")]
    pub average_cost: Option<String>,
    /// The latest price on or before `as_of`; `null` for a sold position.
    pub price: Option<PriceQuoteResponse>,
    /// quantity × price; `0` for a sold position, `null` without a price.
    pub market_value: Option<String>,
    /// Market value − cost basis.
    pub capital_gain: Option<String>,
    /// Capital gain ÷ cost basis × 100, 4 places.
    #[schema(example = "0.4875")]
    pub capital_gain_percent: Option<String>,
    /// Profit or loss of every sale (FIFO, before fees).
    #[schema(example = "582.57")]
    pub realized: String,
    /// Dividends received, after their tax and fee.
    pub dividends: String,
    /// Fees on buys and sells, plus linked `FEE` and `OTHER_EXPENSE` entries.
    pub fees: String,
    /// Taxes on buys and sells, plus linked `TAX` entries, less `TAX_RETURN`.
    pub taxes: String,
    /// Cost of every purchase and opening position, without fees.
    pub total_bought: String,
    /// Capital gain + realized + dividends − fees − taxes.
    #[schema(example = "579.502418825")]
    pub total_profit: Option<String>,
    /// Total profit ÷ total bought × 100, 4 places.
    #[schema(example = "16.1392")]
    pub total_profit_percent: Option<String>,
    /// Why the market value is missing, if it is.
    pub unvalued_reason: Option<UnvaluedReasonDto>,
}

impl From<HoldingReturn> for HoldingReturnResponse {
    fn from(r: HoldingReturn) -> Self {
        let text = |v: Option<rust_decimal::Decimal>| v.map(|v| v.to_string());
        let h = r.holding;
        Self {
            instrument_id: h.instrument_id,
            name: h.name,
            symbol: h.symbol,
            asset_class: h.asset_class.into(),
            quote_currency: h.quote_currency,
            quantity: h.quantity.to_string(),
            cost_basis: h.cost_basis.to_string(),
            average_cost: text(r.average_cost),
            price: r.price.map(Into::into),
            market_value: text(r.market_value),
            capital_gain: text(r.capital_gain),
            capital_gain_percent: text(r.capital_gain_percent),
            realized: h.realized.to_string(),
            dividends: h.dividends.to_string(),
            fees: h.fees.to_string(),
            taxes: h.taxes.to_string(),
            total_bought: h.total_bought.to_string(),
            total_profit: text(r.total_profit),
            total_profit_percent: text(r.total_profit_percent),
            unvalued_reason: r.unvalued.map(Into::into),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReturnsResponse {
    pub portfolio_id: Uuid,
    #[schema(value_type = String, format = Date, example = "2026-11-01")]
    #[serde(serialize_with = "iso_date")]
    pub as_of: Date,
    /// An open position has no price, so its profit is missing.
    pub partial: bool,
    /// Every instrument held on or before `as_of`: open positions first,
    /// then sold ones, each by name.
    pub holdings: Vec<HoldingReturnResponse>,
}

impl From<Returns> for ReturnsResponse {
    fn from(r: Returns) -> Self {
        Self {
            portfolio_id: r.portfolio_id,
            as_of: r.as_of,
            partial: r.partial,
            holdings: r.holdings.into_iter().map(Into::into).collect(),
        }
    }
}

/// Capital gain, realized P&L, dividends, fees, taxes and total profit of
/// every holding at `as_of`, in each instrument's quote currency.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/returns",
    tag = "valuation",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID"), AsOfQuery),
    responses(
        (status = OK, description = "Returns at `as_of`; check `partial`", body = ReturnsResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Missing or invalid `as_of`", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn returns(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(portfolio_id): ResourceId,
    ApiQuery(query): ApiQuery<AsOfQuery>,
) -> Result<Json<ReturnsResponse>, ApiError> {
    let returns = service::returns(
        &state.db,
        session.user.id,
        portfolio_id,
        query.as_of.as_deref(),
    )
    .await?;
    Ok(Json(returns.into()))
}
