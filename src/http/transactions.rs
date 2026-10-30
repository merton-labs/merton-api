//! Ledger transaction endpoints, nested under a portfolio. Decimal values are
//! JSON strings (ADR-API-05); a JSON number is rejected.

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
use crate::app::AppState;
use crate::ledger::service::{self, LedgerEntry, LedgerError};
use crate::ledger::transaction::{RawTransaction, TransactionKind};
use crate::pagination::{Page, Paged};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransactionKindDto {
    OpeningPosition,
    OpeningCash,
    Buy,
    Sell,
    Deposit,
    Withdrawal,
    Dividend,
    Interest,
    Fee,
    Tax,
    TaxReturn,
    CashTransfer,
    FxExchange,
    StockSplit,
    StockDividend,
}

impl From<TransactionKindDto> for TransactionKind {
    fn from(kind: TransactionKindDto) -> Self {
        match kind {
            TransactionKindDto::OpeningPosition => Self::OpeningPosition,
            TransactionKindDto::OpeningCash => Self::OpeningCash,
            TransactionKindDto::Buy => Self::Buy,
            TransactionKindDto::Sell => Self::Sell,
            TransactionKindDto::Deposit => Self::Deposit,
            TransactionKindDto::Withdrawal => Self::Withdrawal,
            TransactionKindDto::Dividend => Self::Dividend,
            TransactionKindDto::Interest => Self::Interest,
            TransactionKindDto::Fee => Self::Fee,
            TransactionKindDto::Tax => Self::Tax,
            TransactionKindDto::TaxReturn => Self::TaxReturn,
            TransactionKindDto::CashTransfer => Self::CashTransfer,
            TransactionKindDto::FxExchange => Self::FxExchange,
            TransactionKindDto::StockSplit => Self::StockSplit,
            TransactionKindDto::StockDividend => Self::StockDividend,
        }
    }
}

impl From<TransactionKind> for TransactionKindDto {
    fn from(kind: TransactionKind) -> Self {
        match kind {
            TransactionKind::OpeningPosition => Self::OpeningPosition,
            TransactionKind::OpeningCash => Self::OpeningCash,
            TransactionKind::Buy => Self::Buy,
            TransactionKind::Sell => Self::Sell,
            TransactionKind::Deposit => Self::Deposit,
            TransactionKind::Withdrawal => Self::Withdrawal,
            TransactionKind::Dividend => Self::Dividend,
            TransactionKind::Interest => Self::Interest,
            TransactionKind::Fee => Self::Fee,
            TransactionKind::Tax => Self::Tax,
            TransactionKind::TaxReturn => Self::TaxReturn,
            TransactionKind::CashTransfer => Self::CashTransfer,
            TransactionKind::FxExchange => Self::FxExchange,
            TransactionKind::StockSplit => Self::StockSplit,
            TransactionKind::StockDividend => Self::StockDividend,
        }
    }
}

/// One transaction. Which fields are required depends on `kind`:
///
/// - `OPENING_POSITION`: `instrument_id`, `quantity`, `amount` (total cost basis of one lot).
/// - `OPENING_CASH`: `cash_account_id`, `amount` (signed starting balance).
/// - `BUY` / `SELL`: `instrument_id`, `quantity`, `price`; optional `fee`, `tax`
///   (default 0) and `cash_account_id` (must use the instrument's quote currency).
/// - `DEPOSIT` / `WITHDRAWAL` / `FEE` / `TAX` / `TAX_RETURN`: `cash_account_id`,
///   `amount` (> 0). A deposit or tax return adds to the account; the others
///   take from it.
/// - `DIVIDEND`: `instrument_id`, `amount` (gross, > 0); optional `fee`, `tax`
///   (withheld, default 0) and `cash_account_id` (credited with the net amount;
///   must use the instrument's quote currency).
/// - `INTEREST`: `cash_account_id`, `amount` (gross, > 0); optional `fee`, `tax`.
///   The account is credited with the net amount.
/// - `CASH_TRANSFER`: `cash_account_id` (source), `destination_cash_account_id`
///   (another account in the same currency), `amount` (> 0); optional `fee`,
///   paid by the source on top of `amount`.
/// - `FX_EXCHANGE`: `cash_account_id` (source), `amount` (spent, > 0),
///   `destination_cash_account_id` (an account in another currency),
///   `destination_amount` (received, > 0); optional `fee` in the source
///   currency, paid on top of `amount`. The executed rate is the ratio of the
///   two amounts.
/// - `STOCK_SPLIT`: `instrument_id`, `ratio` (`new:old`, e.g. `4:1`, or `1:3`
///   for a reverse split). Total cost is unchanged.
/// - `STOCK_DIVIDEND`: `instrument_id`, `quantity` (shares received), spread
///   over the open lots at no cost.
/// - Cash in lieu, for `STOCK_SPLIT` and `STOCK_DIVIDEND`: optional
///   `cash_in_lieu_quantity` (fractional shares, after the action) with
///   `amount` (cash received), and optionally `cash_account_id`. The fraction
///   is disposed of through FIFO, realizing profit or loss.
///
/// For `DIVIDEND` and `INTEREST`, `fee` + `tax` must not exceed `amount`.
/// Fields not listed for a kind are rejected.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TransactionRequest {
    pub kind: TransactionKindDto,
    #[schema(example = "2026-01-15", format = Date)]
    pub trade_date: String,
    /// Orders transactions on the same date (lower first); default 0.
    pub sort_order: Option<i32>,
    pub instrument_id: Option<Uuid>,
    pub cash_account_id: Option<Uuid>,
    /// The receiving account of a `CASH_TRANSFER` or `FX_EXCHANGE`.
    pub destination_cash_account_id: Option<Uuid>,
    #[schema(example = "100", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub quantity: Option<String>,
    /// A `STOCK_SPLIT` ratio, `new:old`.
    #[schema(
        example = "4:1",
        pattern = r"^\d{1,15}(\.\d{1,8})?:\d{1,15}(\.\d{1,8})?$"
    )]
    pub ratio: Option<String>,
    #[schema(example = "35.25", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub price: Option<String>,
    #[schema(example = "15.50", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub fee: Option<String>,
    #[schema(example = "1.09", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub tax: Option<String>,
    #[schema(example = "1000.00", pattern = r"^-?\d{1,15}(\.\d{1,8})?$")]
    pub amount: Option<String>,
    /// The amount an `FX_EXCHANGE` receives, in the destination currency.
    #[schema(example = "82.00", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub destination_amount: Option<String>,
    /// Fractional shares a `STOCK_SPLIT` or `STOCK_DIVIDEND` paid out in cash.
    #[schema(example = "0.5", pattern = r"^\d{1,15}(\.\d{1,8})?$")]
    pub cash_in_lieu_quantity: Option<String>,
    #[schema(max_length = 500)]
    pub note: Option<String>,
}

impl TransactionRequest {
    fn raw(&self) -> RawTransaction<'_> {
        RawTransaction {
            trade_date: &self.trade_date,
            sort_order: self.sort_order,
            instrument_id: self.instrument_id,
            cash_account_id: self.cash_account_id,
            destination_cash_account_id: self.destination_cash_account_id,
            quantity: self.quantity.as_deref(),
            ratio: self.ratio.as_deref(),
            price: self.price.as_deref(),
            fee: self.fee.as_deref(),
            tax: self.tax.as_deref(),
            amount: self.amount.as_deref(),
            destination_amount: self.destination_amount.as_deref(),
            cash_in_lieu_quantity: self.cash_in_lieu_quantity.as_deref(),
            note: self.note.as_deref(),
        }
    }
}

/// A recorded transaction. Decimal values keep the scale they were entered with.
#[derive(Debug, Serialize, ToSchema)]
pub struct TransactionResponse {
    pub id: Uuid,
    pub portfolio_id: Uuid,
    pub kind: TransactionKindDto,
    #[schema(value_type = String, format = Date, example = "2026-01-15")]
    #[serde(serialize_with = "iso_date")]
    pub trade_date: Date,
    pub sort_order: i32,
    pub instrument_id: Option<Uuid>,
    pub cash_account_id: Option<Uuid>,
    pub destination_cash_account_id: Option<Uuid>,
    pub quantity: Option<String>,
    pub ratio: Option<String>,
    pub price: Option<String>,
    pub fee: Option<String>,
    pub tax: Option<String>,
    pub amount: Option<String>,
    pub destination_amount: Option<String>,
    pub cash_in_lieu_quantity: Option<String>,
    pub note: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

pub(super) fn iso_date<S: serde::Serializer>(
    date: &Date,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&format_args!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    ))
}

impl From<LedgerEntry> for TransactionResponse {
    fn from(e: LedgerEntry) -> Self {
        Self {
            id: e.id,
            portfolio_id: e.portfolio_id,
            kind: e.kind.into(),
            trade_date: e.trade_date,
            sort_order: e.sort_order,
            instrument_id: e.instrument_id,
            cash_account_id: e.cash_account_id,
            destination_cash_account_id: e.destination_cash_account_id,
            quantity: e.quantity,
            ratio: e.ratio,
            price: e.price,
            fee: e.fee,
            tax: e.tax,
            amount: e.amount,
            destination_amount: e.destination_amount,
            cash_in_lieu_quantity: e.cash_in_lieu_quantity,
            note: e.note,
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TransactionListResponse {
    pub items: Vec<TransactionResponse>,
    pub total: u64,
    pub limit: u64,
    pub offset: u64,
}

impl From<Paged<LedgerEntry>> for TransactionListResponse {
    fn from(list: Paged<LedgerEntry>) -> Self {
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
pub struct ListTransactionsQuery {
    /// Page size (default 50).
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u64>,
    /// Number of transactions to skip (default 0).
    #[param(minimum = 0)]
    pub offset: Option<u64>,
    /// Only transactions for this instrument.
    pub instrument_id: Option<Uuid>,
}

/// Record a transaction in an active portfolio.
#[utoipa::path(
    post,
    path = "/api/v1/portfolios/{id}/transactions",
    tag = "transactions",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = TransactionRequest,
    responses(
        (status = CREATED, description = "Transaction recorded", body = TransactionResponse,
            headers(("Location" = String, description = "URL of the new transaction"))),
        (status = BAD_REQUEST, description = "Malformed body, unknown field, or a number instead of a decimal string", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Portfolio archived (`portfolio_archived`), holdings would go negative (`insufficient_holdings`), or a split or stock dividend has no holding to adjust (`no_holdings`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid fields or references", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceId(portfolio_id): ResourceId,
    ApiJson(body): ApiJson<TransactionRequest>,
) -> Result<
    (
        StatusCode,
        [(header::HeaderName, String); 1],
        Json<TransactionResponse>,
    ),
    ApiError,
> {
    let entry = service::create(
        &state.db,
        session.user.id,
        portfolio_id,
        body.kind.into(),
        &body.raw(),
    )
    .await?;
    let location = format!(
        "/api/v1/portfolios/{}/transactions/{}",
        entry.portfolio_id, entry.id
    );
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(entry.into()),
    ))
}

/// List a portfolio's transactions in ledger order (date, sort order, entry order).
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/transactions",
    tag = "transactions",
    security(("session_cookie" = [])),
    params(("id" = Uuid, Path, description = "Portfolio ID"), ListTransactionsQuery),
    responses(
        (status = OK, description = "One page of transactions", body = TransactionListResponse),
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
    ApiQuery(query): ApiQuery<ListTransactionsQuery>,
) -> Result<Json<TransactionListResponse>, ApiError> {
    let page = Page::new(query.limit, query.offset)
        .map_err(|field| LedgerError::Validation(vec![field]))?;
    let list = service::list(
        &state.db,
        session.user.id,
        portfolio_id,
        query.instrument_id,
        page,
    )
    .await?;
    Ok(Json(list.into()))
}

/// Get one transaction.
#[utoipa::path(
    get,
    path = "/api/v1/portfolios/{id}/transactions/{transaction_id}",
    tag = "transactions",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("transaction_id" = Uuid, Path, description = "Transaction ID"),
    ),
    responses(
        (status = OK, description = "The transaction", body = TransactionResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio or transaction for this user", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn get(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
    ResourceIds(portfolio_id, id): ResourceIds,
) -> Result<Json<TransactionResponse>, ApiError> {
    let entry = service::get(&state.db, session.user.id, portfolio_id, id).await?;
    Ok(Json(entry.into()))
}

/// Replace a transaction. FIFO history is replayed; the change is rejected if
/// any holding would go negative.
#[utoipa::path(
    put,
    path = "/api/v1/portfolios/{id}/transactions/{transaction_id}",
    tag = "transactions",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("transaction_id" = Uuid, Path, description = "Transaction ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    request_body = TransactionRequest,
    responses(
        (status = OK, description = "Updated transaction", body = TransactionResponse),
        (status = BAD_REQUEST, description = "Malformed body, unknown field, or a number instead of a decimal string", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio or transaction for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Portfolio archived (`portfolio_archived`), holdings would go negative (`insufficient_holdings`), or a split or stock dividend has no holding to adjust (`no_holdings`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNSUPPORTED_MEDIA_TYPE, description = "Body is not JSON", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid fields or references", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn replace(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    ResourceIds(portfolio_id, id): ResourceIds,
    ApiJson(body): ApiJson<TransactionRequest>,
) -> Result<Json<TransactionResponse>, ApiError> {
    let entry = service::replace(
        &state.db,
        session.user.id,
        portfolio_id,
        id,
        body.kind.into(),
        &body.raw(),
    )
    .await?;
    Ok(Json(entry.into()))
}

/// Delete a transaction. Rejected if a later sale would no longer be covered.
#[utoipa::path(
    delete,
    path = "/api/v1/portfolios/{id}/transactions/{transaction_id}",
    tag = "transactions",
    security(("session_cookie" = [])),
    params(
        ("id" = Uuid, Path, description = "Portfolio ID"),
        ("transaction_id" = Uuid, Path, description = "Transaction ID"),
        ("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`"),
    ),
    responses(
        (status = NO_CONTENT, description = "Transaction deleted"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = NOT_FOUND, description = "No such portfolio or transaction for this user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Portfolio archived (`portfolio_archived`), holdings would go negative (`insufficient_holdings`), or a split or stock dividend has no holding to adjust (`no_holdings`)", body = ProblemDetails, content_type = "application/problem+json"),
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
