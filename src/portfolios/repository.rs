//! Owner-scoped portfolio queries. Every function filters by the owner, so a
//! portfolio belonging to another user is invisible here.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::PortfolioStatus;
use crate::auth::UserId;
use crate::infrastructure::entities::portfolio::{self, Column, Entity};

fn owned(owner: UserId) -> Select<Entity> {
    Entity::find().filter(Column::OwnerUserId.eq(owner.as_uuid()))
}

fn with_status(query: Select<Entity>, status: Option<PortfolioStatus>) -> Select<Entity> {
    match status {
        Some(status) => query.filter(Column::Status.eq(status.as_str())),
        None => query,
    }
}

pub async fn insert(db: &impl ConnectionTrait, model: portfolio::Model) -> Result<(), DbErr> {
    let active = portfolio::ActiveModel {
        id: Set(model.id),
        owner_user_id: Set(model.owner_user_id),
        name: Set(model.name),
        base_currency: Set(model.base_currency),
        status: Set(model.status),
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
) -> Result<Option<portfolio::Model>, DbErr> {
    owned(owner).filter(Column::Id.eq(id)).one(db).await
}

/// One page in creation order; `id` breaks ties so pages never overlap.
pub async fn list(
    db: &impl ConnectionTrait,
    owner: UserId,
    status: Option<PortfolioStatus>,
    limit: u64,
    offset: u64,
) -> Result<Vec<portfolio::Model>, DbErr> {
    with_status(owned(owner), status)
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
    status: Option<PortfolioStatus>,
) -> Result<u64, DbErr> {
    with_status(owned(owner), status).count(db).await
}

/// Updates an owned portfolio only while it is `ACTIVE`. Returns the number
/// of rows changed, so the check and the write cannot race.
pub async fn update_active(
    db: &impl ConnectionTrait,
    owner: UserId,
    id: Uuid,
    name: Option<&str>,
    base_currency: Option<&str>,
    now: OffsetDateTime,
) -> Result<u64, DbErr> {
    let mut update = Entity::update_many().col_expr(Column::UpdatedAt, Expr::value(now));
    if let Some(name) = name {
        update = update.col_expr(Column::Name, Expr::value(name));
    }
    if let Some(base_currency) = base_currency {
        update = update.col_expr(Column::BaseCurrency, Expr::value(base_currency));
    }
    let result = update
        .filter(Column::OwnerUserId.eq(owner.as_uuid()))
        .filter(Column::Id.eq(id))
        .filter(Column::Status.eq(PortfolioStatus::Active.as_str()))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// Moves an owned portfolio from `from` to `to`; returns rows changed.
pub async fn transition(
    db: &impl ConnectionTrait,
    owner: UserId,
    id: Uuid,
    from: PortfolioStatus,
    to: PortfolioStatus,
    now: OffsetDateTime,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value(to.as_str()))
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .filter(Column::OwnerUserId.eq(owner.as_uuid()))
        .filter(Column::Id.eq(id))
        .filter(Column::Status.eq(from.as_str()))
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
