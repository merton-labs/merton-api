//! Private portfolios owned by a single user.
//!
//! Every operation is scoped to the owner's [`UserId`](crate::auth::UserId),
//! which only comes from a verified session. A portfolio owned by someone
//! else is reported exactly like a missing one.

mod repository;
pub mod service;

use sea_orm::DbErr;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::FieldError;

pub const MAX_NAME_CHARS: usize = 100;
pub const DEFAULT_PAGE_SIZE: u64 = 50;
pub const MAX_PAGE_SIZE: u64 = 200;
/// Largest offset the database can bind (SQL offsets are signed 64-bit).
pub const MAX_OFFSET: u64 = i64::MAX as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortfolioStatus {
    Active,
    Archived,
}

impl PortfolioStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Archived => "ARCHIVED",
        }
    }

    fn from_db(value: &str) -> Result<Self, DbErr> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "ARCHIVED" => Ok(Self::Archived),
            other => Err(DbErr::Type(format!("unknown portfolio status {other:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portfolio {
    pub id: Uuid,
    pub name: String,
    pub base_currency: String,
    pub status: PortfolioStatus,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A validated display name: trimmed, 1–100 characters, no control characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioName(String);

impl PortfolioName {
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        let name = input.trim();
        if name.is_empty() {
            Err("must not be empty")
        } else if name.chars().count() > MAX_NAME_CHARS {
            Err("must be at most 100 characters")
        } else if name.chars().any(char::is_control) {
            Err("must not contain control characters")
        } else {
            Ok(Self(name.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A three-letter currency code, normalized to uppercase.
///
/// Only the format is checked; codes are not matched against the ISO 4217
/// list (see docs/PROJECT_PLAN.md §22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        let code = input.trim();
        if code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic()) {
            Ok(Self(code.to_ascii_uppercase()))
        } else {
            Err("must be a three-letter currency code such as USD or THB")
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bounded offset pagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub limit: u64,
    pub offset: u64,
}

impl Page {
    pub fn new(limit: Option<u64>, offset: Option<u64>) -> Result<Self, FieldError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            return Err(FieldError {
                field: "limit",
                message: format!("must be between 1 and {MAX_PAGE_SIZE}"),
            });
        }
        let offset = offset.unwrap_or(0);
        if offset > MAX_OFFSET {
            return Err(FieldError {
                field: "offset",
                message: format!("must be at most {MAX_OFFSET}"),
            });
        }
        Ok(Self { limit, offset })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioList {
    pub items: Vec<Portfolio>,
    pub total: u64,
    pub page: Page,
}

#[derive(Debug, thiserror::Error)]
pub enum PortfolioError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("portfolio not found")]
    NotFound,
    #[error("portfolio is archived")]
    Archived,
    #[error("portfolio is not archived")]
    NotArchived,
    #[error("portfolio has financial history")]
    HasFinancialHistory,
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(
            PortfolioName::parse("  Retirement  ").unwrap().as_str(),
            "Retirement"
        );
        assert_eq!(
            PortfolioName::parse(&"é".repeat(100)).map(|n| n.0.chars().count()),
            Ok(100)
        );
        assert!(PortfolioName::parse("   ").is_err());
        assert!(PortfolioName::parse(&"x".repeat(101)).is_err());
        assert!(PortfolioName::parse("line\nbreak").is_err());
    }

    #[test]
    fn currency_codes_are_three_letters_uppercased() {
        assert_eq!(CurrencyCode::parse(" thb ").unwrap().as_str(), "THB");
        for bad in ["", "US", "USDX", "U$D", "123", "ÜSD"] {
            assert!(CurrencyCode::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn page_defaults_and_bounds() {
        assert_eq!(
            Page::new(None, None).unwrap(),
            Page {
                limit: 50,
                offset: 0
            }
        );
        assert_eq!(Page::new(Some(200), Some(7)).unwrap().offset, 7);
        assert!(Page::new(Some(0), None).is_err());
        assert!(Page::new(Some(201), None).is_err());
        assert!(Page::new(None, Some(MAX_OFFSET)).is_ok());
        assert!(Page::new(None, Some(MAX_OFFSET + 1)).is_err());
    }

    #[test]
    fn status_round_trips_through_storage_form() {
        for status in [PortfolioStatus::Active, PortfolioStatus::Archived] {
            assert_eq!(PortfolioStatus::from_db(status.as_str()).unwrap(), status);
        }
        assert!(PortfolioStatus::from_db("DELETED").is_err());
    }
}
