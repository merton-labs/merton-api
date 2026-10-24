//! Owner-scoped instrument queries. Every function filters by the owner.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::AssetClass;
use crate::auth::UserId;
use crate::infrastructure::entities::instrument::{self, Column, Entity};

/// Changes to apply; `None` leaves a column untouched, `Some(None)` clears it.
pub struct Changes<'a> {
    pub asset_class: Option<AssetClass>,
    pub name: Option<&'a str>,
    pub symbol: Option<Option<&'a str>>,
    pub isin: Option<Option<&'a str>>,
    pub exchange: Option<Option<&'a str>>,
}

fn owned(owner: UserId) -> Select<Entity> {
    Entity::find().filter(Column::OwnerUserId.eq(owner.as_uuid()))
}

fn with_class(query: Select<Entity>, class: Option<AssetClass>) -> Select<Entity> {
    match class {
        Some(class) => query.filter(Column::AssetClass.eq(class.as_str())),
        None => query,
    }
}

pub async fn insert(db: &impl ConnectionTrait, model: instrument::Model) -> Result<(), DbErr> {
    let active = instrument::ActiveModel {
        id: Set(model.id),
        owner_user_id: Set(model.owner_user_id),
        asset_class: Set(model.asset_class),
        name: Set(model.name),
        symbol: Set(model.symbol),
        isin: Set(model.isin),
        exchange: Set(model.exchange),
        quote_currency: Set(model.quote_currency),
        created_at: Set(model.created_at),
        updated_at: Set(model.updated_at),
    };
    Entity::insert(active).exec_without_returning(db).await?;
    Ok(())
}

pub async fn find(
    db: &impl ConnectionTrait,
    owner: UserId,
    id: Uuid,
) -> Result<Option<instrument::Model>, DbErr> {
    owned(owner).filter(Column::Id.eq(id)).one(db).await
}

pub async fn list(
    db: &impl ConnectionTrait,
    owner: UserId,
    class: Option<AssetClass>,
    limit: u64,
    offset: u64,
) -> Result<Vec<instrument::Model>, DbErr> {
    with_class(owned(owner), class)
        .order_by_asc(Column::CreatedAt)
        .order_by_asc(Column::Id)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await
}

pub async fn count(
    db: &impl ConnectionTrait,
    owner: UserId,
    class: Option<AssetClass>,
) -> Result<u64, DbErr> {
    with_class(owned(owner), class).count(db).await
}

pub async fn update(
    db: &impl ConnectionTrait,
    owner: UserId,
    id: Uuid,
    changes: Changes<'_>,
    now: OffsetDateTime,
) -> Result<u64, DbErr> {
    let mut update = Entity::update_many().col_expr(Column::UpdatedAt, Expr::value(now));
    if let Some(class) = changes.asset_class {
        update = update.col_expr(Column::AssetClass, Expr::value(class.as_str()));
    }
    if let Some(name) = changes.name {
        update = update.col_expr(Column::Name, Expr::value(name));
    }
    for (column, value) in [
        (Column::Symbol, changes.symbol),
        (Column::Isin, changes.isin),
        (Column::Exchange, changes.exchange),
    ] {
        if let Some(value) = value {
            update = update.col_expr(column, Expr::value(value.map(str::to_owned)));
        }
    }
    let result = update
        .filter(Column::OwnerUserId.eq(owner.as_uuid()))
        .filter(Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn delete(db: &impl ConnectionTrait, owner: UserId, id: Uuid) -> Result<u64, DbErr> {
    let result = Entity::delete_many()
        .filter(Column::OwnerUserId.eq(owner.as_uuid()))
        .filter(Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}
