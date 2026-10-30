//! Historical market data for valuation: instrument prices and FX rates.
//!
//! Both are entered by the user and never fetched or fabricated
//! (ADR-VAL-02, VAL-03). A price is in its instrument's quote currency. An
//! FX rate says how many units of `quote_currency` one unit of
//! `base_currency` is worth on `rate_date`.

pub(crate) mod repository;
pub mod service;

use sea_orm::DbErr;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::auth::FieldError;
use crate::ledger::decimal::{Price, Rate};
use crate::pagination::Paged;

/// Where a price or rate came from (ADR-VAL-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Entered through the API.
    Manual,
    /// Imported from CSV (P6).
    Import,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "MANUAL",
            Self::Import => "IMPORT",
        }
    }

    fn from_db(value: &str) -> Result<Self, DbErr> {
        match value {
            "MANUAL" => Ok(Self::Manual),
            "IMPORT" => Ok(Self::Import),
            other => Err(DbErr::Type(format!("unknown market data source {other:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentPrice {
    pub id: Uuid,
    pub instrument_id: Uuid,
    pub price_date: Date,
    pub price: Price,
    pub source: Source,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxRate {
    pub id: Uuid,
    pub base_currency: String,
    pub quote_currency: String,
    pub rate_date: Date,
    pub rate: Rate,
    pub source: Source,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub type PriceList = Paged<InstrumentPrice>;
pub type FxRateList = Paged<FxRate>;

#[derive(Debug, thiserror::Error)]
pub enum MarketDataError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("not found")]
    NotFound,
    #[error("the instrument already has a price on this date")]
    PriceExists,
    #[error("a rate for this pair already exists on this date")]
    FxRateExists,
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_round_trips_through_storage_form() {
        for source in [Source::Manual, Source::Import] {
            assert_eq!(Source::from_db(source.as_str()).unwrap(), source);
        }
        assert!(Source::from_db("FEED").is_err());
    }
}
