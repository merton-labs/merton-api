//! RFC 9457 Problem Details responses.

use std::time::Duration;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

use crate::auth::{AuthError, FieldError};
use crate::cash_accounts::CashAccountError;
use crate::instruments::InstrumentError;
use crate::ledger::service::LedgerError;
use crate::market_data::MarketDataError;
use crate::portfolios::PortfolioError;

const PROBLEM_JSON: &str = "application/problem+json";
/// Argon2 operations finish in well under a second, so capacity frees quickly.
const OVERLOAD_RETRY_AFTER_SECS: u64 = 1;

/// Rounds up so a client that waits exactly `Retry-After` is not rejected again.
fn whole_seconds_ceil(duration: Duration) -> u64 {
    let seconds = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);
    seconds.max(1)
}

/// An RFC 9457 problem document.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemDetails {
    /// Problem type URI; `about:blank` means the HTTP status says it all.
    #[serde(rename = "type")]
    #[schema(example = "about:blank")]
    pub problem_type: &'static str,
    #[schema(example = "Unauthorized")]
    pub title: &'static str,
    #[schema(example = 401)]
    pub status: u16,
    #[schema(example = "Invalid email or password.")]
    pub detail: String,
    /// Stable machine-readable error code.
    #[schema(example = "invalid_credentials")]
    pub code: &'static str,
    /// Per-field problems, present for validation errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ProblemField>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemField {
    #[schema(example = "email")]
    pub field: &'static str,
    #[schema(example = "must be a valid email address")]
    pub message: String,
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    UnsupportedMediaType(String),
    Unauthenticated,
    InvalidCredentials,
    Forbidden {
        code: &'static str,
        detail: &'static str,
    },
    NotFound,
    EmailTaken,
    Conflict {
        code: &'static str,
        detail: &'static str,
    },
    InsufficientHoldings(String),
    NoHoldings(String),
    Validation(Vec<FieldError>),
    RateLimited(Duration),
    Overloaded,
    Internal,
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            Self::BadRequest(detail) => {
                (StatusCode::BAD_REQUEST, "malformed_request", detail.clone())
            }
            Self::UnsupportedMediaType(detail) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                detail.clone(),
            ),
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "A valid session is required.".to_owned(),
            ),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Invalid email or password.".to_owned(),
            ),
            Self::Forbidden { code, detail } => (StatusCode::FORBIDDEN, code, (*detail).to_owned()),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "No resource exists at this path.".to_owned(),
            ),
            Self::Conflict { code, detail } => (StatusCode::CONFLICT, code, (*detail).to_owned()),
            Self::InsufficientHoldings(detail) => (
                StatusCode::CONFLICT,
                "insufficient_holdings",
                detail.clone(),
            ),
            Self::NoHoldings(detail) => (StatusCode::CONFLICT, "no_holdings", detail.clone()),
            Self::EmailTaken => (
                StatusCode::CONFLICT,
                "email_taken",
                "An account with this email already exists.".to_owned(),
            ),
            Self::Validation(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                "The request contains invalid fields.".to_owned(),
            ),
            Self::RateLimited(_) => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many authentication attempts. Try again later.".to_owned(),
            ),
            Self::Overloaded => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_overloaded",
                "The server is busy processing other sign-ins. Try again shortly.".to_owned(),
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "An unexpected error occurred.".to_owned(),
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, detail) = self.parts();
        let errors = match &self {
            Self::Validation(fields) => Some(
                fields
                    .iter()
                    .map(|f| ProblemField {
                        field: f.field,
                        message: f.message.clone(),
                    })
                    .collect(),
            ),
            _ => None,
        };
        let body = ProblemDetails {
            problem_type: "about:blank",
            title: status.canonical_reason().unwrap_or("Error"),
            status: status.as_u16(),
            detail,
            code,
            errors,
        };

        let mut response = (status, Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        let retry_after = match self {
            Self::RateLimited(remaining) => Some(whole_seconds_ceil(remaining)),
            Self::Overloaded => Some(OVERLOAD_RETRY_AFTER_SECS),
            _ => None,
        };
        if let Some(seconds) = retry_after {
            headers.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

impl From<AuthError> for ApiError {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::Validation(fields) => Self::Validation(fields),
            AuthError::EmailTaken => Self::EmailTaken,
            AuthError::InvalidCredentials => Self::InvalidCredentials,
            AuthError::RateLimited(retry_after) => Self::RateLimited(retry_after.0),
            AuthError::Unauthenticated => Self::Unauthenticated,
            AuthError::Overloaded => Self::Overloaded,
            AuthError::Internal(err) => {
                tracing::error!(error = %err, "internal error");
                Self::Internal
            }
        }
    }
}

impl From<PortfolioError> for ApiError {
    fn from(err: PortfolioError) -> Self {
        match err {
            PortfolioError::Validation(fields) => Self::Validation(fields),
            PortfolioError::NotFound => Self::NotFound,
            PortfolioError::Archived => Self::Conflict {
                code: "portfolio_archived",
                detail: "The portfolio is archived. Restore it before changing it.",
            },
            PortfolioError::NotArchived => Self::Conflict {
                code: "portfolio_not_archived",
                detail: "The portfolio is not archived.",
            },
            PortfolioError::HasFinancialHistory => Self::Conflict {
                code: "portfolio_has_history",
                detail: "The portfolio has financial history and cannot be deleted.",
            },
            PortfolioError::Database(err) => {
                tracing::error!(error = %err, "portfolio database error");
                Self::Internal
            }
        }
    }
}

impl From<InstrumentError> for ApiError {
    fn from(err: InstrumentError) -> Self {
        match err {
            InstrumentError::Validation(fields) => Self::Validation(fields),
            InstrumentError::NotFound => Self::NotFound,
            InstrumentError::InUse => Self::Conflict {
                code: "instrument_in_use",
                detail: "The instrument is referenced by other records and cannot be deleted.",
            },
            InstrumentError::Database(err) => {
                tracing::error!(error = %err, "instrument database error");
                Self::Internal
            }
        }
    }
}

impl From<MarketDataError> for ApiError {
    fn from(err: MarketDataError) -> Self {
        match err {
            MarketDataError::Validation(fields) => Self::Validation(fields),
            MarketDataError::NotFound => Self::NotFound,
            MarketDataError::PriceExists => Self::Conflict {
                code: "price_exists",
                detail: "The instrument already has a price on this date. Change that price instead.",
            },
            MarketDataError::FxRateExists => Self::Conflict {
                code: "fx_rate_exists",
                detail: "A rate for this currency pair already exists on this date. Change that rate instead.",
            },
            MarketDataError::Database(err) => {
                tracing::error!(error = %err, "market data database error");
                Self::Internal
            }
        }
    }
}

impl From<CashAccountError> for ApiError {
    fn from(err: CashAccountError) -> Self {
        match err {
            CashAccountError::Validation(fields) => Self::Validation(fields),
            CashAccountError::NotFound => Self::NotFound,
            CashAccountError::PortfolioArchived => Self::Conflict {
                code: "portfolio_archived",
                detail: "The portfolio is archived. Restore it before changing it.",
            },
            CashAccountError::InUse => Self::Conflict {
                code: "cash_account_in_use",
                detail: "The cash account has recorded movements and cannot be deleted.",
            },
            CashAccountError::Database(err) => {
                tracing::error!(error = %err, "cash account database error");
                Self::Internal
            }
        }
    }
}

impl From<LedgerError> for ApiError {
    fn from(err: LedgerError) -> Self {
        match err {
            LedgerError::Validation(fields) => Self::Validation(fields),
            LedgerError::NotFound => Self::NotFound,
            LedgerError::PortfolioArchived => Self::Conflict {
                code: "portfolio_archived",
                detail: "The portfolio is archived. Restore it before changing it.",
            },
            LedgerError::InsufficientHoldings {
                instrument_id,
                trade_date,
            } => Self::InsufficientHoldings(format!(
                "Holdings of instrument {instrument_id} would go negative on {trade_date}. \
                 Sell no more than is held at each point in history."
            )),
            LedgerError::NoHoldings {
                instrument_id,
                trade_date,
            } => Self::NoHoldings(format!(
                "Instrument {instrument_id} has no holding on {trade_date} for a split or \
                 stock dividend to adjust."
            )),
            LedgerError::Database(err) => {
                tracing::error!(error = %err, "ledger database error");
                Self::Internal
            }
        }
    }
}

pub async fn not_found() -> ApiError {
    ApiError::NotFound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        assert_eq!(whole_seconds_ceil(Duration::from_millis(1)), 1);
        assert_eq!(whole_seconds_ceil(Duration::from_secs(1)), 1);
        assert_eq!(whole_seconds_ceil(Duration::from_millis(899_400)), 900);
        assert_eq!(whole_seconds_ceil(Duration::ZERO), 1);
    }
}
