//! Cash accounts inside a portfolio.
//!
//! A portfolio may hold many cash accounts, several in the same currency;
//! each account has exactly one currency, fixed at creation (ADR-LEDG-05).
//! Bank deposits are represented as cash accounts (ADR-INST-04).
//!
//! Accounts are owned through their portfolio: every query requires the
//! portfolio to belong to the caller, and changes require it to be `ACTIVE`.

pub(crate) mod repository;
pub mod service;

use sea_orm::DbErr;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::FieldError;
use crate::pagination::Paged;

pub const MAX_NAME_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashAccount {
    pub id: Uuid,
    pub portfolio_id: Uuid,
    pub name: String,
    pub currency: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub type CashAccountList = Paged<CashAccount>;

#[derive(Debug, thiserror::Error)]
pub enum CashAccountError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    /// The portfolio or the account does not exist for this user.
    #[error("cash account not found")]
    NotFound,
    #[error("portfolio is archived")]
    PortfolioArchived,
    #[error("cash account is referenced by other records")]
    InUse,
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}
