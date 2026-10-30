//! Market data queries. Every one is owner-scoped in the same statement:
//! FX rates by their owner column, prices by an `EXISTS` guard requiring the
//! instrument to belong to the owner.

use sea_orm::sea_query::{Expr, Query};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select,
};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::auth::UserId;
use crate::infrastructure::entities::{fx_rate, instrument_price};
use crate::instruments::repository::owned_instrument;

/// An inclusive date range; `None` leaves that end open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DateRange {
    pub from: Option<Date>,
    pub to: Option<Date>,
}

/// FX rate list filters; `None` matches anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FxRateFilter {
    pub base_currency: Option<String>,
    pub quote_currency: Option<String>,
    pub dates: DateRange,
}

/// The editable values of a price or rate.
pub struct PriceValues<'a> {
    pub price_date: Date,
    pub price: &'a str,
}

pub struct FxRateValues<'a> {
    pub base_currency: &'a str,
    pub quote_currency: &'a str,
    pub rate_date: Date,
    pub rate: &'a str,
}

// --- Prices ------------------------------------------------------------------

mod price {
    pub use crate::infrastructure::entities::instrument_price::{Column, Entity};
}

fn prices_of_owned(owner: UserId, instrument_id: Uuid) -> Select<price::Entity> {
    price::Entity::find()
        .filter(price::Column::InstrumentId.eq(instrument_id))
        .filter(Expr::exists(owned_instrument(owner, instrument_id)))
}

fn in_range<E: EntityTrait>(
    mut query: Select<E>,
    column: impl ColumnTrait,
    range: DateRange,
) -> Select<E> {
    if let Some(from) = range.from {
        query = query.filter(column.gte(from));
    }
    if let Some(to) = range.to {
        query = query.filter(column.lte(to));
    }
    query
}

/// Inserts the price only if its instrument belongs to the owner; returns
/// the number of rows inserted.
pub async fn insert_price(
    db: &impl ConnectionTrait,
    owner: UserId,
    model: instrument_price::Model,
) -> Result<u64, DbErr> {
    let mut values = owned_instrument(owner, model.instrument_id);
    values.clear_selects().exprs([
        Expr::value(model.id),
        Expr::value(model.instrument_id),
        Expr::value(model.price_date),
        Expr::value(model.price),
        Expr::value(model.source),
        Expr::value(model.created_at),
        Expr::value(model.updated_at),
    ]);
    let mut insert = Query::insert();
    insert
        .into_table(price::Entity)
        .columns([
            price::Column::Id,
            price::Column::InstrumentId,
            price::Column::PriceDate,
            price::Column::Price,
            price::Column::Source,
            price::Column::CreatedAt,
            price::Column::UpdatedAt,
        ])
        .select_from(values)
        .map_err(|err| DbErr::Custom(err.to_string()))?;
    Ok(db.execute(&insert).await?.rows_affected())
}

pub async fn find_price(
    db: &impl ConnectionTrait,
    owner: UserId,
    instrument_id: Uuid,
    id: Uuid,
) -> Result<Option<instrument_price::Model>, DbErr> {
    prices_of_owned(owner, instrument_id)
        .filter(price::Column::Id.eq(id))
        .one(db)
        .await
}

/// Latest first.
pub async fn list_prices(
    db: &impl ConnectionTrait,
    owner: UserId,
    instrument_id: Uuid,
    range: DateRange,
    limit: u64,
    offset: u64,
) -> Result<Vec<instrument_price::Model>, DbErr> {
    in_range(
        prices_of_owned(owner, instrument_id),
        price::Column::PriceDate,
        range,
    )
    .order_by_desc(price::Column::PriceDate)
    .limit(limit)
    .offset(offset)
    .all(db)
    .await
}

pub async fn count_prices(
    db: &impl ConnectionTrait,
    owner: UserId,
    instrument_id: Uuid,
    range: DateRange,
) -> Result<u64, DbErr> {
    in_range(
        prices_of_owned(owner, instrument_id),
        price::Column::PriceDate,
        range,
    )
    .count(db)
    .await
}

pub async fn update_price(
    db: &impl ConnectionTrait,
    owner: UserId,
    instrument_id: Uuid,
    id: Uuid,
    values: PriceValues<'_>,
    now: OffsetDateTime,
) -> Result<u64, DbErr> {
    let result = price::Entity::update_many()
        .col_expr(price::Column::PriceDate, Expr::value(values.price_date))
        .col_expr(price::Column::Price, Expr::value(values.price))
        .col_expr(price::Column::UpdatedAt, Expr::value(now))
        .filter(price::Column::Id.eq(id))
        .filter(price::Column::InstrumentId.eq(instrument_id))
        .filter(Expr::exists(owned_instrument(owner, instrument_id)))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn delete_price(
    db: &impl ConnectionTrait,
    owner: UserId,
    instrument_id: Uuid,
    id: Uuid,
) -> Result<u64, DbErr> {
    let result = price::Entity::delete_many()
        .filter(price::Column::Id.eq(id))
        .filter(price::Column::InstrumentId.eq(instrument_id))
        .filter(Expr::exists(owned_instrument(owner, instrument_id)))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

// --- FX rates ----------------------------------------------------------------

mod rate {
    pub use crate::infrastructure::entities::fx_rate::{Column, Entity};
}

fn rates_of(owner: UserId) -> Select<rate::Entity> {
    rate::Entity::find().filter(rate::Column::OwnerUserId.eq(owner.as_uuid()))
}

fn filtered(owner: UserId, filter: &FxRateFilter) -> Select<rate::Entity> {
    let mut query = in_range(rates_of(owner), rate::Column::RateDate, filter.dates);
    if let Some(base) = &filter.base_currency {
        query = query.filter(rate::Column::BaseCurrency.eq(base.as_str()));
    }
    if let Some(quote) = &filter.quote_currency {
        query = query.filter(rate::Column::QuoteCurrency.eq(quote.as_str()));
    }
    query
}

pub async fn insert_fx_rate(db: &impl ConnectionTrait, model: fx_rate::Model) -> Result<(), DbErr> {
    let active = fx_rate::ActiveModel {
        id: Set(model.id),
        owner_user_id: Set(model.owner_user_id),
        base_currency: Set(model.base_currency),
        quote_currency: Set(model.quote_currency),
        rate_date: Set(model.rate_date),
        rate: Set(model.rate),
        source: Set(model.source),
        created_at: Set(model.created_at),
        updated_at: Set(model.updated_at),
    };
    rate::Entity::insert(active)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

pub async fn find_fx_rate(
    db: &impl ConnectionTrait,
    owner: UserId,
    id: Uuid,
) -> Result<Option<fx_rate::Model>, DbErr> {
    rates_of(owner)
        .filter(rate::Column::Id.eq(id))
        .one(db)
        .await
}

/// Latest first, then by pair.
pub async fn list_fx_rates(
    db: &impl ConnectionTrait,
    owner: UserId,
    filter: &FxRateFilter,
    limit: u64,
    offset: u64,
) -> Result<Vec<fx_rate::Model>, DbErr> {
    filtered(owner, filter)
        .order_by_desc(rate::Column::RateDate)
        .order_by_asc(rate::Column::BaseCurrency)
        .order_by_asc(rate::Column::QuoteCurrency)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await
}

pub async fn count_fx_rates(
    db: &impl ConnectionTrait,
    owner: UserId,
    filter: &FxRateFilter,
) -> Result<u64, DbErr> {
    filtered(owner, filter).count(db).await
}

pub async fn update_fx_rate(
    db: &impl ConnectionTrait,
    owner: UserId,
    id: Uuid,
    values: FxRateValues<'_>,
    now: OffsetDateTime,
) -> Result<u64, DbErr> {
    let result = rate::Entity::update_many()
        .col_expr(
            rate::Column::BaseCurrency,
            Expr::value(values.base_currency),
        )
        .col_expr(
            rate::Column::QuoteCurrency,
            Expr::value(values.quote_currency),
        )
        .col_expr(rate::Column::RateDate, Expr::value(values.rate_date))
        .col_expr(rate::Column::Rate, Expr::value(values.rate))
        .col_expr(rate::Column::UpdatedAt, Expr::value(now))
        .filter(rate::Column::OwnerUserId.eq(owner.as_uuid()))
        .filter(rate::Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn delete_fx_rate(
    db: &impl ConnectionTrait,
    owner: UserId,
    id: Uuid,
) -> Result<u64, DbErr> {
    let result = rate::Entity::delete_many()
        .filter(rate::Column::OwnerUserId.eq(owner.as_uuid()))
        .filter(rate::Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}
