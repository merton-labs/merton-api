//! Price and FX-rate use cases.
//!
//! Each statement is owner-scoped and runs on its own, so no read-then-write
//! transaction is needed. Duplicates are caught by the unique indexes, so a
//! concurrent insert of the same date cannot slip through.

use sea_orm::{DatabaseConnection, DbErr, SqlErr, TransactionTrait};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::repository::{self, DateRange, FxRateFilter, FxRateValues, PriceValues};
use super::{FxRate, FxRateList, InstrumentPrice, MarketDataError, PriceList, Source};
use crate::auth::{FieldError, UserId};
use crate::infrastructure::database::is_foreign_key_violation;
use crate::infrastructure::entities::{fx_rate, instrument_price};
use crate::instruments::repository as instruments;
use crate::ledger::decimal::{Price, Rate};
use crate::ledger::transaction::parse_date;
use crate::pagination::Page;
use crate::validation::{CurrencyCode, check};

pub struct PriceInput<'a> {
    pub price_date: &'a str,
    pub price: &'a str,
}

pub struct FxRateInput<'a> {
    pub base_currency: &'a str,
    pub quote_currency: &'a str,
    pub rate_date: &'a str,
    pub rate: &'a str,
}

/// Optional inclusive date bounds, as entered.
#[derive(Default)]
pub struct DateQuery<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
}

#[derive(Default)]
pub struct FxRateQuery<'a> {
    pub base_currency: Option<&'a str>,
    pub quote_currency: Option<&'a str>,
    pub dates: DateQuery<'a>,
}

// --- Prices ------------------------------------------------------------------

pub async fn create_price(
    db: &DatabaseConnection,
    owner: UserId,
    instrument_id: Uuid,
    input: PriceInput<'_>,
) -> Result<InstrumentPrice, MarketDataError> {
    let (price_date, price) = validate_price(&input)?;
    let now = OffsetDateTime::now_utc();
    let model = instrument_price::Model {
        id: Uuid::now_v7(),
        instrument_id,
        price_date,
        price: price.to_string(),
        source: Source::Manual.as_str().to_owned(),
        created_at: now,
        updated_at: now,
    };
    match repository::insert_price(db, owner, model.clone()).await {
        Ok(0) => Err(MarketDataError::NotFound),
        Ok(_) => price_to_domain(model),
        Err(err) => Err(price_write_error(err)),
    }
}

pub async fn list_prices(
    db: &DatabaseConnection,
    owner: UserId,
    instrument_id: Uuid,
    query: DateQuery<'_>,
    page: Page,
) -> Result<PriceList, MarketDataError> {
    let range = validate_range(&query)?;
    let txn = db.begin().await?;
    if instruments::find(&txn, owner, instrument_id)
        .await?
        .is_none()
    {
        return Err(MarketDataError::NotFound);
    }
    let total = repository::count_prices(&txn, owner, instrument_id, range).await?;
    let models =
        repository::list_prices(&txn, owner, instrument_id, range, page.limit, page.offset).await?;
    txn.commit().await?;
    let items = models
        .into_iter()
        .map(price_to_domain)
        .collect::<Result<_, _>>()?;
    Ok(PriceList { items, total, page })
}

pub async fn get_price(
    db: &DatabaseConnection,
    owner: UserId,
    instrument_id: Uuid,
    id: Uuid,
) -> Result<InstrumentPrice, MarketDataError> {
    repository::find_price(db, owner, instrument_id, id)
        .await?
        .ok_or(MarketDataError::NotFound)
        .and_then(price_to_domain)
}

/// Replaces a price's date and value; its source is kept.
pub async fn replace_price(
    db: &DatabaseConnection,
    owner: UserId,
    instrument_id: Uuid,
    id: Uuid,
    input: PriceInput<'_>,
) -> Result<InstrumentPrice, MarketDataError> {
    let (price_date, price) = validate_price(&input)?;
    let values = PriceValues {
        price_date,
        price: &price.to_string(),
    };
    let now = OffsetDateTime::now_utc();
    match repository::update_price(db, owner, instrument_id, id, values, now).await {
        Ok(0) => Err(MarketDataError::NotFound),
        Ok(_) => get_price(db, owner, instrument_id, id).await,
        Err(err) => Err(price_write_error(err)),
    }
}

pub async fn delete_price(
    db: &DatabaseConnection,
    owner: UserId,
    instrument_id: Uuid,
    id: Uuid,
) -> Result<(), MarketDataError> {
    match repository::delete_price(db, owner, instrument_id, id).await? {
        0 => Err(MarketDataError::NotFound),
        _ => Ok(()),
    }
}

fn validate_price(input: &PriceInput<'_>) -> Result<(Date, Price), MarketDataError> {
    let mut errors = Vec::new();
    let date = check(&mut errors, "price_date", parse_date(input.price_date));
    let price = check(&mut errors, "price", Price::parse(input.price));
    match (date, price) {
        (Some(date), Some(price)) => Ok((date, price)),
        _ => Err(MarketDataError::Validation(errors)),
    }
}

/// The instrument can vanish between the guard and the insert only through
/// a concurrent delete, which is the same as it never having existed.
fn price_write_error(err: DbErr) -> MarketDataError {
    if is_unique_violation(&err) {
        MarketDataError::PriceExists
    } else if is_foreign_key_violation(&err) {
        MarketDataError::NotFound
    } else {
        err.into()
    }
}

fn price_to_domain(model: instrument_price::Model) -> Result<InstrumentPrice, MarketDataError> {
    let corrupt = |err: String| DbErr::Type(format!("price {}: {err}", model.id));
    Ok(InstrumentPrice {
        id: model.id,
        instrument_id: model.instrument_id,
        price_date: model.price_date,
        price: Price::from_stored(&model.price).map_err(corrupt)?,
        source: Source::from_db(&model.source)?,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}

// --- FX rates ----------------------------------------------------------------

pub async fn create_fx_rate(
    db: &DatabaseConnection,
    owner: UserId,
    input: FxRateInput<'_>,
) -> Result<FxRate, MarketDataError> {
    let valid = validate_fx_rate(&input)?;
    let now = OffsetDateTime::now_utc();
    let model = fx_rate::Model {
        id: Uuid::now_v7(),
        owner_user_id: owner.as_uuid(),
        base_currency: valid.base.as_str().to_owned(),
        quote_currency: valid.quote.as_str().to_owned(),
        rate_date: valid.date,
        rate: valid.rate.to_string(),
        source: Source::Manual.as_str().to_owned(),
        created_at: now,
        updated_at: now,
    };
    match repository::insert_fx_rate(db, model.clone()).await {
        Ok(()) => fx_rate_to_domain(model),
        Err(err) => Err(fx_rate_write_error(err)),
    }
}

pub async fn list_fx_rates(
    db: &DatabaseConnection,
    owner: UserId,
    query: FxRateQuery<'_>,
    page: Page,
) -> Result<FxRateList, MarketDataError> {
    let mut errors = Vec::new();
    let base = query
        .base_currency
        .and_then(|c| check(&mut errors, "base_currency", CurrencyCode::parse(c)));
    let quote = query
        .quote_currency
        .and_then(|c| check(&mut errors, "quote_currency", CurrencyCode::parse(c)));
    let dates = match validate_range(&query.dates) {
        Ok(dates) => dates,
        Err(MarketDataError::Validation(mut more)) => {
            errors.append(&mut more);
            DateRange::default()
        }
        Err(err) => return Err(err),
    };
    if !errors.is_empty() {
        return Err(MarketDataError::Validation(errors));
    }
    let filter = FxRateFilter {
        base_currency: base.map(|c| c.as_str().to_owned()),
        quote_currency: quote.map(|c| c.as_str().to_owned()),
        dates,
    };

    let txn = db.begin().await?;
    let total = repository::count_fx_rates(&txn, owner, &filter).await?;
    let models = repository::list_fx_rates(&txn, owner, &filter, page.limit, page.offset).await?;
    txn.commit().await?;
    let items = models
        .into_iter()
        .map(fx_rate_to_domain)
        .collect::<Result<_, _>>()?;
    Ok(FxRateList { items, total, page })
}

pub async fn get_fx_rate(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<FxRate, MarketDataError> {
    repository::find_fx_rate(db, owner, id)
        .await?
        .ok_or(MarketDataError::NotFound)
        .and_then(fx_rate_to_domain)
}

pub async fn replace_fx_rate(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
    input: FxRateInput<'_>,
) -> Result<FxRate, MarketDataError> {
    let valid = validate_fx_rate(&input)?;
    let values = FxRateValues {
        base_currency: valid.base.as_str(),
        quote_currency: valid.quote.as_str(),
        rate_date: valid.date,
        rate: &valid.rate.to_string(),
    };
    let now = OffsetDateTime::now_utc();
    match repository::update_fx_rate(db, owner, id, values, now).await {
        Ok(0) => Err(MarketDataError::NotFound),
        Ok(_) => get_fx_rate(db, owner, id).await,
        Err(err) => Err(fx_rate_write_error(err)),
    }
}

pub async fn delete_fx_rate(
    db: &DatabaseConnection,
    owner: UserId,
    id: Uuid,
) -> Result<(), MarketDataError> {
    match repository::delete_fx_rate(db, owner, id).await? {
        0 => Err(MarketDataError::NotFound),
        _ => Ok(()),
    }
}

struct ValidFxRate {
    base: CurrencyCode,
    quote: CurrencyCode,
    date: Date,
    rate: Rate,
}

fn validate_fx_rate(input: &FxRateInput<'_>) -> Result<ValidFxRate, MarketDataError> {
    let mut errors = Vec::new();
    let base = check(
        &mut errors,
        "base_currency",
        CurrencyCode::parse(input.base_currency),
    );
    let quote = check(
        &mut errors,
        "quote_currency",
        CurrencyCode::parse(input.quote_currency),
    );
    let date = check(&mut errors, "rate_date", parse_date(input.rate_date));
    let rate = check(&mut errors, "rate", Rate::parse(input.rate));
    if let (Some(base), Some(quote)) = (&base, &quote)
        && base == quote
    {
        errors.push(FieldError {
            field: "quote_currency",
            message: "must differ from base_currency".to_owned(),
        });
    }
    match (base, quote, date, rate) {
        (Some(base), Some(quote), Some(date), Some(rate)) if errors.is_empty() => Ok(ValidFxRate {
            base,
            quote,
            date,
            rate,
        }),
        _ => Err(MarketDataError::Validation(errors)),
    }
}

fn fx_rate_write_error(err: DbErr) -> MarketDataError {
    if is_unique_violation(&err) {
        MarketDataError::FxRateExists
    } else {
        err.into()
    }
}

fn fx_rate_to_domain(model: fx_rate::Model) -> Result<FxRate, MarketDataError> {
    let corrupt = |err: String| DbErr::Type(format!("fx rate {}: {err}", model.id));
    Ok(FxRate {
        id: model.id,
        base_currency: model.base_currency,
        quote_currency: model.quote_currency,
        rate_date: model.rate_date,
        rate: Rate::from_stored(&model.rate).map_err(corrupt)?,
        source: Source::from_db(&model.source)?,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}

// --- Shared ------------------------------------------------------------------

fn validate_range(query: &DateQuery<'_>) -> Result<DateRange, MarketDataError> {
    let mut errors = Vec::new();
    let from = query
        .from
        .and_then(|d| check(&mut errors, "from", parse_date(d)));
    let to = query
        .to
        .and_then(|d| check(&mut errors, "to", parse_date(d)));
    if let (Some(from), Some(to)) = (from, to)
        && to < from
    {
        errors.push(FieldError {
            field: "to",
            message: "must not be before from".to_owned(),
        });
    }
    if errors.is_empty() {
        Ok(DateRange { from, to })
    } else {
        Err(MarketDataError::Validation(errors))
    }
}

fn is_unique_violation(err: &DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}
