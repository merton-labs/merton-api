//! Cash account use cases.

use sea_orm::{DatabaseConnection, TransactionTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use super::repository;
use super::{CashAccount, CashAccountError, CashAccountList, MAX_NAME_CHARS};
use crate::auth::UserId;
use crate::infrastructure::database::is_foreign_key_violation;
use crate::infrastructure::entities::cash_account;
use crate::pagination::Page;
use crate::portfolios::PortfolioStatus;
use crate::portfolios::repository as portfolios;
use crate::validation::{CurrencyCode, DisplayName, check};

pub async fn create(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    name: &str,
    currency: &str,
) -> Result<CashAccount, CashAccountError> {
    let mut errors = Vec::new();
    let name = check(
        &mut errors,
        "name",
        DisplayName::parse(name, MAX_NAME_CHARS),
    );
    let currency = check(&mut errors, "currency", CurrencyCode::parse(currency));
    let (Some(name), Some(currency)) = (name, currency) else {
        return Err(CashAccountError::Validation(errors));
    };

    let now = OffsetDateTime::now_utc();
    let model = cash_account::Model {
        id: Uuid::now_v7(),
        portfolio_id,
        name: name.as_str().to_owned(),
        currency: currency.as_str().to_owned(),
        created_at: now,
        updated_at: now,
    };
    if repository::insert_if_active(db, owner, model.clone()).await? == 0 {
        // The portfolio is missing, or was not active at write time.
        return Err(match portfolio_state(db, owner, portfolio_id).await? {
            None => CashAccountError::NotFound,
            Some(_) => CashAccountError::PortfolioArchived,
        });
    }
    Ok(to_domain(model))
}

pub async fn list(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    page: Page,
) -> Result<CashAccountList, CashAccountError> {
    let txn = db.begin().await?;
    if portfolios::find(&txn, owner, portfolio_id).await?.is_none() {
        return Err(CashAccountError::NotFound);
    }
    let total = repository::count(&txn, owner, portfolio_id).await?;
    let models = repository::list(&txn, owner, portfolio_id, page.limit, page.offset).await?;
    txn.commit().await?;
    Ok(CashAccountList {
        items: models.into_iter().map(to_domain).collect(),
        total,
        page,
    })
}

pub async fn get(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
) -> Result<CashAccount, CashAccountError> {
    repository::find(db, owner, portfolio_id, id)
        .await?
        .map(to_domain)
        .ok_or(CashAccountError::NotFound)
}

/// Renames an account. Its currency is fixed at creation.
pub async fn rename(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
    name: &str,
) -> Result<CashAccount, CashAccountError> {
    let name = DisplayName::parse(name, MAX_NAME_CHARS).map_err(|message| {
        CashAccountError::Validation(vec![crate::auth::FieldError {
            field: "name",
            message,
        }])
    })?;
    let changed = repository::rename_if_active(
        db,
        owner,
        portfolio_id,
        id,
        name.as_str(),
        OffsetDateTime::now_utc(),
    )
    .await?;
    if changed == 0 {
        return Err(explain_no_change(db, owner, portfolio_id).await?);
    }
    get(db, owner, portfolio_id, id).await
}

/// Deletes an account with no history. Ledger tables reference cash
/// accounts with `ON DELETE RESTRICT`, so an account in use is refused with
/// [`CashAccountError::InUse`].
pub async fn delete(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
) -> Result<(), CashAccountError> {
    match repository::delete_if_active(db, owner, portfolio_id, id).await {
        Ok(0) => Err(explain_no_change(db, owner, portfolio_id).await?),
        Ok(_) => Ok(()),
        Err(err) if is_foreign_key_violation(&err) => Err(CashAccountError::InUse),
        Err(err) => Err(err.into()),
    }
}

/// After a guarded write on an existing account matched nothing.
async fn explain_no_change(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
) -> Result<CashAccountError, CashAccountError> {
    Ok(match portfolio_state(db, owner, portfolio_id).await? {
        Some(PortfolioStatus::Archived) => CashAccountError::PortfolioArchived,
        // Missing portfolio, or an active portfolio without this account.
        _ => CashAccountError::NotFound,
    })
}

async fn portfolio_state(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
) -> Result<Option<PortfolioStatus>, CashAccountError> {
    let portfolio = portfolios::find(db, owner, portfolio_id).await?;
    Ok(portfolio.map(|p| {
        if p.status == PortfolioStatus::Archived.as_str() {
            PortfolioStatus::Archived
        } else {
            PortfolioStatus::Active
        }
    }))
}

fn to_domain(model: cash_account::Model) -> CashAccount {
    CashAccount {
        id: model.id,
        portfolio_id: model.portfolio_id,
        name: model.name,
        currency: model.currency,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}
