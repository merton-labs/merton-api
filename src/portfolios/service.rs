//! Portfolio use cases.
//!
//! Lifecycle (ADR-PORT-03): `ACTIVE` portfolios can be read, updated and
//! archived; `ARCHIVED` ones can be read, restored or deleted, but not
//! updated.

use sea_orm::{DatabaseConnection, TransactionTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use super::repository;
use super::{
    CurrencyCode, Page, Portfolio, PortfolioError, PortfolioList, PortfolioName, PortfolioStatus,
};
use crate::auth::{FieldError, UserId};
use crate::infrastructure::database::is_foreign_key_violation;
use crate::infrastructure::entities::portfolio;

pub struct PortfolioChanges<'a> {
    pub name: Option<&'a str>,
    pub base_currency: Option<&'a str>,
}

pub async fn create(
    db: &DatabaseConnection,
    owner: UserId,
    name: &str,
    base_currency: &str,
) -> Result<Portfolio, PortfolioError> {
    let mut errors = Vec::new();
    let name = check(&mut errors, "name", PortfolioName::parse(name));
    let currency = check(
        &mut errors,
        "base_currency",
        CurrencyCode::parse(base_currency),
    );
    let (Some(name), Some(currency)) = (name, currency) else {
        return Err(PortfolioError::Validation(errors));
    };

    let now = OffsetDateTime::now_utc();
    let model = portfolio::Model {
        id: Uuid::now_v7(),
        owner_user_id: owner.as_uuid(),
        name: name.as_str().to_owned(),
        base_currency: currency.as_str().to_owned(),
        status: PortfolioStatus::Active.as_str().to_owned(),
        created_at: now,
        updated_at: now,
    };
    repository::insert(db, model.clone()).await?;
    to_domain(model)
}

pub async fn list(
    db: &DatabaseConnection,
    owner: UserId,
    status: Option<PortfolioStatus>,
    page: Page,
) -> Result<PortfolioList, PortfolioError> {
    // One read transaction so `total` and `items` come from the same snapshot.
    let txn = db.begin().await?;
    let total = repository::count(&txn, owner, status).await?;
    let models = repository::list(&txn, owner, status, page.limit, page.offset).await?;
    txn.commit().await?;
    let items = models
        .into_iter()
        .map(to_domain)
        .collect::<Result<_, _>>()?;
    Ok(PortfolioList { items, total, page })
}

pub async fn get(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<Portfolio, PortfolioError> {
    repository::find(db, owner, id)
        .await?
        .ok_or(PortfolioError::NotFound)
        .and_then(to_domain)
}

/// Renames and/or changes the base currency of an active portfolio.
///
/// Changing the base currency only affects reporting; it never alters
/// recorded transactions (ADR-PORT-02).
pub async fn update(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
    changes: PortfolioChanges<'_>,
) -> Result<Portfolio, PortfolioError> {
    let mut errors = Vec::new();
    let name = changes
        .name
        .and_then(|n| check(&mut errors, "name", PortfolioName::parse(n)));
    let currency = changes
        .base_currency
        .and_then(|c| check(&mut errors, "base_currency", CurrencyCode::parse(c)));
    if changes.name.is_none() && changes.base_currency.is_none() {
        errors.push(FieldError {
            field: "body",
            message: "at least one of name or base_currency is required".to_owned(),
        });
    }
    if !errors.is_empty() {
        return Err(PortfolioError::Validation(errors));
    }

    let changed = repository::update_active(
        db,
        owner,
        id,
        name.as_ref().map(PortfolioName::as_str),
        currency.as_ref().map(CurrencyCode::as_str),
        OffsetDateTime::now_utc(),
    )
    .await?;
    if changed == 0 {
        return Err(missing_or(db, owner, id, PortfolioError::Archived).await);
    }
    get(db, owner, id).await
}

pub async fn archive(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<Portfolio, PortfolioError> {
    transition(
        db,
        owner,
        id,
        PortfolioStatus::Active,
        PortfolioStatus::Archived,
        PortfolioError::Archived,
    )
    .await
}

pub async fn restore(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<Portfolio, PortfolioError> {
    transition(
        db,
        owner,
        id,
        PortfolioStatus::Archived,
        PortfolioStatus::Active,
        PortfolioError::NotArchived,
    )
    .await
}

/// Permanently deletes a portfolio that has no financial history
/// (ADR-PORT-04).
///
/// Financial tables must reference portfolios with immediate (not
/// deferrable) `ON DELETE RESTRICT`. The database then refuses to delete a
/// portfolio that has history, and this returns
/// [`PortfolioError::HasFinancialHistory`]; records are never cascaded away.
///
/// That foreign-key check is a backstop: it cannot tell financial history
/// from other references. When a phase adds a table that references
/// portfolios, it must also add an explicit check here, and decide whether
/// rows that are not history (such as an unused cash account) are removed
/// in the same transaction or block deletion.
pub async fn delete(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<(), PortfolioError> {
    match repository::delete(db, owner, id).await {
        Ok(0) => Err(PortfolioError::NotFound),
        Ok(_) => Ok(()),
        Err(err) if is_foreign_key_violation(&err) => Err(PortfolioError::HasFinancialHistory),
        Err(err) => Err(err.into()),
    }
}

async fn transition(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
    from: PortfolioStatus,
    to: PortfolioStatus,
    wrong_state: PortfolioError,
) -> Result<Portfolio, PortfolioError> {
    let changed =
        repository::transition(db, owner, id, from, to, OffsetDateTime::now_utc()).await?;
    if changed == 0 {
        return Err(missing_or(db, owner, id, wrong_state).await);
    }
    get(db, owner, id).await
}

/// After a conditional write matched nothing: `NotFound` if the owner has no
/// such portfolio, otherwise `wrong_state`.
async fn missing_or(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
    wrong_state: PortfolioError,
) -> PortfolioError {
    match repository::find(db, owner, id).await {
        Ok(Some(_)) => wrong_state,
        Ok(None) => PortfolioError::NotFound,
        Err(err) => err.into(),
    }
}

fn check<T>(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    result: Result<T, &'static str>,
) -> Option<T> {
    result
        .map_err(|message| {
            errors.push(FieldError {
                field,
                message: message.to_owned(),
            })
        })
        .ok()
}

fn to_domain(model: portfolio::Model) -> Result<Portfolio, PortfolioError> {
    Ok(Portfolio {
        id: model.id,
        status: PortfolioStatus::from_db(&model.status)?,
        name: model.name,
        base_currency: model.base_currency,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}
