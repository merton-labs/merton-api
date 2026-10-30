//! User-scoped instruments: the securities a user holds or tracks.
//!
//! Identity attributes (symbol, ISIN, exchange) are optional and not unique,
//! because tickers and listings are ambiguous (ADR-INST-01). The quote
//! currency is fixed when the instrument is created (ADR-INST-03).

pub(crate) mod repository;
pub mod service;

use sea_orm::DbErr;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::FieldError;
use crate::pagination::Paged;

pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_CODE_CHARS: usize = 32;

/// Instrument asset classes (ADR-INST-04). Cash is held in cash accounts,
/// not as an instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Stock,
    Etf,
    MutualFund,
}

impl AssetClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stock => "STOCK",
            Self::Etf => "ETF",
            Self::MutualFund => "MUTUAL_FUND",
        }
    }

    pub(crate) fn from_db(value: &str) -> Result<Self, DbErr> {
        match value {
            "STOCK" => Ok(Self::Stock),
            "ETF" => Ok(Self::Etf),
            "MUTUAL_FUND" => Ok(Self::MutualFund),
            other => Err(DbErr::Type(format!("unknown asset class {other:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instrument {
    pub id: Uuid,
    pub asset_class: AssetClass,
    pub name: String,
    pub symbol: Option<String>,
    pub isin: Option<String>,
    pub exchange: Option<String>,
    pub quote_currency: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub type InstrumentList = Paged<Instrument>;

/// An optional short code such as a ticker or exchange. Blank input means
/// "no value"; otherwise it is trimmed and kept as entered.
pub fn optional_code(input: Option<&str>) -> Result<Option<String>, String> {
    let Some(code) = input.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    if code.chars().count() > MAX_CODE_CHARS {
        Err(format!("must be at most {MAX_CODE_CHARS} characters"))
    } else if code.chars().any(char::is_control) {
        Err("must not contain control characters".to_owned())
    } else {
        Ok(Some(code.to_owned()))
    }
}

/// An optional ISIN (ISO 6166): two letters, nine alphanumerics and a check
/// digit, validated and stored uppercase. Blank input means "no value".
pub fn optional_isin(input: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = input.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let isin = raw.to_ascii_uppercase();
    let bytes = isin.as_bytes();
    let well_formed = bytes.len() == 12
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..11].iter().all(u8::is_ascii_alphanumeric)
        && bytes[11].is_ascii_digit();
    if !well_formed {
        return Err("must be a 12-character ISIN such as US0378331005".to_owned());
    }
    if !isin_check_digit_is_valid(bytes) {
        return Err("has an invalid check digit".to_owned());
    }
    Ok(Some(isin))
}

/// Luhn check over the ISIN with letters expanded to two digits (A=10 … Z=35).
fn isin_check_digit_is_valid(isin: &[u8]) -> bool {
    let digits: Vec<u32> = isin
        .iter()
        .flat_map(|&b| {
            let value = if b.is_ascii_digit() {
                u32::from(b - b'0')
            } else {
                u32::from(b - b'A') + 10
            };
            if value >= 10 {
                vec![value / 10, value % 10]
            } else {
                vec![value]
            }
        })
        .collect();
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let doubled = d * 2;
                doubled / 10 + doubled % 10
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

#[derive(Debug, thiserror::Error)]
pub enum InstrumentError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("instrument not found")]
    NotFound,
    #[error("instrument is referenced by other records")]
    InUse,
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_isins_case_insensitively() {
        for isin in [
            "US0378331005",
            "us5949181045",
            "GB0002634946",
            " AU0000XVGZA3 ",
        ] {
            assert!(optional_isin(Some(isin)).is_ok(), "{isin}");
        }
        assert_eq!(
            optional_isin(Some("us0378331005")),
            Ok(Some("US0378331005".to_owned()))
        );
    }

    #[test]
    fn rejects_malformed_isins_and_bad_check_digits() {
        assert_eq!(
            optional_isin(Some("US0378331006")),
            Err("has an invalid check digit".to_owned())
        );
        for bad in [
            "US037833100",
            "US03783310055",
            "1S0378331005",
            "US037833100X",
            "US03783-1005",
        ] {
            assert!(optional_isin(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn blank_identifiers_mean_none() {
        assert_eq!(optional_isin(None), Ok(None));
        assert_eq!(optional_isin(Some("  ")), Ok(None));
        assert_eq!(optional_code(Some("")), Ok(None));
        assert_eq!(optional_code(Some(" BRK.B ")), Ok(Some("BRK.B".to_owned())));
        assert!(optional_code(Some(&"X".repeat(33))).is_err());
        assert!(optional_code(Some("A\tB")).is_err());
    }

    #[test]
    fn asset_class_round_trips_through_storage_form() {
        for class in [AssetClass::Stock, AssetClass::Etf, AssetClass::MutualFund] {
            assert_eq!(AssetClass::from_db(class.as_str()).unwrap(), class);
        }
        assert!(AssetClass::from_db("BOND").is_err());
    }
}
