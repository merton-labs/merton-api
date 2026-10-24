//! Private portfolios owned by a single user.
//!
//! Every operation is scoped to the owner's [`UserId`](crate::auth::UserId),
//! which only comes from a verified session. A portfolio owned by someone
//! else is reported exactly like a missing one.

pub(crate) mod repository;
pub mod service;

use sea_orm::DbErr;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::FieldError;
use crate::pagination::Paged;

pub const MAX_NAME_CHARS: usize = 100;

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

pub type PortfolioList = Paged<Portfolio>;

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
    fn status_round_trips_through_storage_form() {
        for status in [PortfolioStatus::Active, PortfolioStatus::Archived] {
            assert_eq!(PortfolioStatus::from_db(status.as_str()).unwrap(), status);
        }
        assert!(PortfolioStatus::from_db("DELETED").is_err());
    }
}
