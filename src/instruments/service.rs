//! Instrument use cases.

use sea_orm::{DatabaseConnection, TransactionTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use super::repository::{self, Changes};
use super::{
    AssetClass, Instrument, InstrumentError, InstrumentList, MAX_NAME_CHARS, optional_code,
    optional_isin,
};
use crate::auth::{FieldError, UserId};
use crate::infrastructure::database::is_foreign_key_violation;
use crate::infrastructure::entities::instrument;
use crate::pagination::Page;
use crate::validation::{CurrencyCode, DisplayName, check};

pub struct NewInstrument<'a> {
    pub asset_class: AssetClass,
    pub name: &'a str,
    pub symbol: Option<&'a str>,
    pub isin: Option<&'a str>,
    pub exchange: Option<&'a str>,
    pub quote_currency: &'a str,
}

/// `None` leaves a field unchanged; `Some(None)` clears an optional field.
pub struct InstrumentChanges<'a> {
    pub asset_class: Option<AssetClass>,
    pub name: Option<&'a str>,
    pub symbol: Option<Option<&'a str>>,
    pub isin: Option<Option<&'a str>>,
    pub exchange: Option<Option<&'a str>>,
}

pub async fn create(
    db: &DatabaseConnection,
    owner: UserId,
    new: NewInstrument<'_>,
) -> Result<Instrument, InstrumentError> {
    let mut errors = Vec::new();
    let name = check(
        &mut errors,
        "name",
        DisplayName::parse(new.name, MAX_NAME_CHARS),
    );
    let symbol = check(&mut errors, "symbol", optional_code(new.symbol));
    let isin = check(&mut errors, "isin", optional_isin(new.isin));
    let exchange = check(&mut errors, "exchange", optional_code(new.exchange));
    let currency = check(
        &mut errors,
        "quote_currency",
        CurrencyCode::parse(new.quote_currency),
    );
    let (Some(name), Some(symbol), Some(isin), Some(exchange), Some(currency)) =
        (name, symbol, isin, exchange, currency)
    else {
        return Err(InstrumentError::Validation(errors));
    };

    let now = OffsetDateTime::now_utc();
    let model = instrument::Model {
        id: Uuid::now_v7(),
        owner_user_id: owner.as_uuid(),
        asset_class: new.asset_class.as_str().to_owned(),
        name: name.as_str().to_owned(),
        symbol,
        isin,
        exchange,
        quote_currency: currency.as_str().to_owned(),
        created_at: now,
        updated_at: now,
    };
    repository::insert(db, model.clone()).await?;
    to_domain(model)
}

pub async fn list(
    db: &DatabaseConnection,
    owner: UserId,
    class: Option<AssetClass>,
    page: Page,
) -> Result<InstrumentList, InstrumentError> {
    let txn = db.begin().await?;
    let total = repository::count(&txn, owner, class).await?;
    let models = repository::list(&txn, owner, class, page.limit, page.offset).await?;
    txn.commit().await?;
    let items = models
        .into_iter()
        .map(to_domain)
        .collect::<Result<_, _>>()?;
    Ok(InstrumentList { items, total, page })
}

pub async fn get(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<Instrument, InstrumentError> {
    repository::find(db, owner, id)
        .await?
        .ok_or(InstrumentError::NotFound)
        .and_then(to_domain)
}

/// Updates descriptive attributes. The quote currency cannot change: prices
/// and transactions recorded later depend on it (ADR-INST-03).
pub async fn update(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
    changes: InstrumentChanges<'_>,
) -> Result<Instrument, InstrumentError> {
    let mut errors = Vec::new();
    let name = changes
        .name
        .and_then(|n| check(&mut errors, "name", DisplayName::parse(n, MAX_NAME_CHARS)));
    let symbol = changes
        .symbol
        .and_then(|s| check(&mut errors, "symbol", optional_code(s)));
    let isin = changes
        .isin
        .and_then(|i| check(&mut errors, "isin", optional_isin(i)));
    let exchange = changes
        .exchange
        .and_then(|e| check(&mut errors, "exchange", optional_code(e)));
    let nothing_requested = changes.asset_class.is_none()
        && changes.name.is_none()
        && changes.symbol.is_none()
        && changes.isin.is_none()
        && changes.exchange.is_none();
    if nothing_requested {
        errors.push(FieldError {
            field: "body",
            message: "at least one field to change is required".to_owned(),
        });
    }
    if !errors.is_empty() {
        return Err(InstrumentError::Validation(errors));
    }

    let changed = repository::update(
        db,
        owner,
        id,
        Changes {
            asset_class: changes.asset_class,
            name: name.as_ref().map(DisplayName::as_str),
            symbol: symbol.as_ref().map(Option::as_deref),
            isin: isin.as_ref().map(Option::as_deref),
            exchange: exchange.as_ref().map(Option::as_deref),
        },
        OffsetDateTime::now_utc(),
    )
    .await?;
    if changed == 0 {
        return Err(InstrumentError::NotFound);
    }
    get(db, owner, id).await
}

/// Deletes an instrument nothing refers to. Later tables (transactions,
/// prices) reference instruments with `ON DELETE RESTRICT`, so an instrument
/// in use is refused with [`InstrumentError::InUse`].
pub async fn delete(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<(), InstrumentError> {
    match repository::delete(db, owner, id).await {
        Ok(0) => Err(InstrumentError::NotFound),
        Ok(_) => Ok(()),
        Err(err) if is_foreign_key_violation(&err) => Err(InstrumentError::InUse),
        Err(err) => Err(err.into()),
    }
}

fn to_domain(model: instrument::Model) -> Result<Instrument, InstrumentError> {
    Ok(Instrument {
        id: model.id,
        asset_class: AssetClass::from_db(&model.asset_class)?,
        name: model.name,
        symbol: model.symbol,
        isin: model.isin,
        exchange: model.exchange,
        quote_currency: model.quote_currency,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}
