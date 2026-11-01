//! Dividends and interest received, computed on request (ADR-DIV-01).
//! Reads work on archived portfolios too.

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use time::Date;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{ApiQuery, CurrentSession, ResourceId};
use super::instruments::AssetClassDto;
use super::transactions::{TransactionKindDto, iso_date};
use super::valuation::{AllocationClassDto, UnvaluedReasonDto};
use crate::app::AppState;
use crate::dividends::{Dividends, Source, service};

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DividendsQuery {
    /// First payment date to include (`YYYY-MM-DD`); from the start when
    /// omitted.
    pub from: Option<String>,
    /// Last payment date to include (`YYYY-MM-DD`); up to the latest when
    /// omitted.
    pub to: Option<String>,
}

fn date_text(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DividendSourceKindDto {
    /// A dividend from a holding.
    Instrument,
    /// Interest paid on a cash account.
    CashAccount,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DividendSourceResponse {
    pub kind: DividendSourceKindDto,
    /// The instrument or cash account ID.
    pub id: Uuid,
    pub name: String,
    pub symbol: Option<String>,
    /// `null` for a cash account.
    pub asset_class: Option<AssetClassDto>,
    /// The currency the payments were made in.
    pub currency: String,
    /// Received, in the base currency (decimal string).
    #[schema(example = "727.92")]
    pub received: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DividendMonthResponse {
    /// Calendar month, `YYYY-MM`.
    #[schema(example = "2026-06")]
    pub month: String,
    /// Received, in the base currency (decimal string).
    #[schema(example = "342.58")]
    pub received: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DividendClassResponse {
    /// `CASH` for interest on cash accounts.
    pub asset_class: AllocationClassDto,
    pub received: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DividendCurrencyResponse {
    /// The currency the payments were made in.
    pub currency: String,
    /// Their total in the base currency.
    pub received: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnconvertedPaymentResponse {
    pub transaction_id: Uuid,
    pub kind: TransactionKindDto,
    #[schema(value_type = String, format = Date)]
    #[serde(serialize_with = "iso_date")]
    pub trade_date: Date,
    pub source_id: Uuid,
    pub name: String,
    pub currency: String,
    /// Received, in `currency` (decimal string).
    pub received: String,
    pub reason: UnvaluedReasonDto,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DividendsResponse {
    pub portfolio_id: Uuid,
    pub base_currency: String,
    /// The `from` filter, if given.
    #[schema(value_type = Option<String>, format = Date)]
    pub from: Option<String>,
    /// The `to` filter, if given.
    #[schema(value_type = Option<String>, format = Date)]
    pub to: Option<String>,
    /// Some payment had no FX rate and is missing from every total.
    pub partial: bool,
    /// Received in the base currency, after tax and fees.
    #[schema(example = "1466.33790000")]
    pub total: String,
    /// Months with a payment, oldest first.
    pub months: Vec<DividendMonthResponse>,
    /// By holding or cash account, largest first.
    pub sources: Vec<DividendSourceResponse>,
    /// By asset class.
    pub classes: Vec<DividendClassResponse>,
    /// By payment currency, largest first.
    pub currencies: Vec<DividendCurrencyResponse>,
    /// Payments left out of the totals, in ledger order.
    pub unconverted: Vec<UnconvertedPaymentResponse>,
}

fn source_parts(source: &Source) -> (DividendSourceKindDto, Uuid, String) {
    match source {
        Source::Instrument { id, name, .. } => {
            (DividendSourceKindDto::Instrument, *id, name.clone())
        }
        Source::CashAccount { id, name } => (DividendSourceKindDto::CashAccount, *id, name.clone()),
    }
}

impl From<Dividends> for DividendsResponse {
    fn from(d: Dividends) -> Self {
        Self {
            portfolio_id: d.portfolio_id,
            base_currency: d.base_currency,
            from: d.from.map(date_text),
            to: d.to.map(date_text),
            partial: d.partial,
            total: d.total.to_string(),
            months: d
                .months
                .into_iter()
                .map(|m| DividendMonthResponse {
                    month: format!("{:04}-{:02}", m.year, u8::from(m.month)),
                    received: m.received.to_string(),
                })
                .collect(),
            sources: d
                .sources
                .into_iter()
                .map(|s| {
                    let (kind, id, name) = source_parts(&s.source);
                    let (symbol, asset_class) = match s.source {
                        Source::Instrument {
                            symbol,
                            asset_class,
                            ..
                        } => (symbol, Some(asset_class.into())),
                        Source::CashAccount { .. } => (None, None),
                    };
                    DividendSourceResponse {
                        kind,
                        id,
                        name,
                        symbol,
                        asset_class,
                        currency: s.currency,
                        received: s.received.to_string(),
                    }
                })
                .collect(),
            classes: d
                .classes
                .into_iter()
                .map(|c| DividendClassResponse {
                    asset_class: c.class.into(),
                    received: c.received.to_string(),
                })
                .collect(),
            currencies: d
                .currencies
                .into_iter()
                .map(|c| DividendCurrencyResponse {
                    currency: c.currency,
                    received: c.received.to_string(),
                })
                .collect(),
            unconverted: d
                .unconverted
                .into_iter()
                .map(|u| {
                    let (_, source_id, name) = source_parts(&u.payment.source);
                    UnconvertedPaymentResponse {
                        transaction_id: u.payment.transaction_id,
                        kind: u.payment.kind.into(),
                        trade_date: u.payment.trade_date,
                        source_id,
                        name,
                        currency: u.payment.currency,
                        received: u.payment.received.to_string(),
                        reason: u.reason.into(),
                    }
                })
                .collect(),
        }
    }
}

/// Dividends and interest received, after tax and fees, in the base
/// currency. Each payment is converted at the latest FX rate on or before
/// its own date.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/dividends",
    tag = "dividends",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID"), DividendsQuery),
    responses(
        (status = OK, description = "Dividends received; check `partial`", body = DividendsResponse),
        (status = BAD_REQUEST, description = "Malformed query", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid `from` or `to`, `to` before `from`, or totals too large", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn dividends(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceId(portfolio_id): ResourceId,
    ApiQuery(query): ApiQuery<DividendsQuery>,
) -> Result<Json<DividendsResponse>, ApiError> {
    let dividends = service::dividends(
        &state.db,
        session.user.id,
        portfolio_id,
        query.from.as_deref(),
        query.to.as_deref(),
    )
    .await?;
    Ok(Json(dividends.into()))
}
